use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use csv::Reader;
use dotenv::dotenv;
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::fs::File;

#[derive(Debug, Clone, Deserialize)]
struct OutageRecord {
    #[serde(rename = "Date")]
    date: String,
    #[serde(rename = "Ticket")]
    ticket: String,
    #[serde(rename = "CloudStack/Service")]
    service: String,
    #[serde(rename = "Duration (in minutes)")]
    duration: String,
    #[serde(rename = "Cause")]
    cause: String,
    #[serde(rename = "Solution")]
    solution: String,
    #[serde(rename = "Severity")]
    severity: String,
}

#[derive(Debug, Deserialize)]
struct JiraIssue {
    fields: JiraFields,
}

#[derive(Debug, Deserialize)]
struct JiraFields {
    description: Option<String>,
}


fn parse_date(date_str: &str) -> Option<NaiveDate> {
    let parts: Vec<&str> = date_str.split('/').collect();
    if parts.len() != 3 {
        return None;
    }

    let day = parts[0].parse::<u32>().ok()?;
    let month_str = parts[1];
    let year = parts[2].parse::<i32>().ok().map(|y| 2000 + y)?;

    let month = match month_str {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };

    NaiveDate::from_ymd_opt(year, month, day)
}

fn get_previous_week_range() -> (NaiveDate, NaiveDate) {
    let today = Local::now().date_naive();
    let days_since_sunday = match today.weekday() {
        Weekday::Sun => 0,
        Weekday::Mon => 1,
        Weekday::Tue => 2,
        Weekday::Wed => 3,
        Weekday::Thu => 4,
        Weekday::Fri => 5,
        Weekday::Sat => 6,
    };

    let last_sunday = today - Duration::days(days_since_sunday as i64);
    let previous_sunday = last_sunday - Duration::days(7);
    let previous_saturday = previous_sunday + Duration::days(6);

    (previous_sunday, previous_saturday)
}

fn extract_jira_key(url: &str) -> Option<String> {
    // Extract JIRA key pattern like OPS-12345 from the URL
    let parts: Vec<&str> = url.split('/').collect();
    for part in parts {
        // Check if this part matches JIRA key pattern (LETTERS-NUMBERS)
        if part.contains('-') {
            let key_parts: Vec<&str> = part.split('-').collect();
            if key_parts.len() == 2 {
                if key_parts[0].chars().all(|c| c.is_ascii_uppercase()) &&
                   key_parts[1].chars().all(|c| c.is_ascii_digit()) {
                    return Some(part.to_string());
                }
            }
        }
    }
    None
}

async fn fetch_jira_details(
    jira_key: &str,
    email: &str,
    token: &str,
) -> Result<JiraIssue, Box<dyn Error>> {
    let client = reqwest::Client::new();
    let url = format!("https://sugarcrm.atlassian.net/rest/api/2/issue/{}", jira_key);

    // Try Bearer token first
    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/json")
        .send()
        .await?;

    let response = if !response.status().is_success() {
        // If Bearer fails, try Basic auth with email:token
        let auth_string = format!("{}:{}", email, token);
        let encoded_auth = STANDARD.encode(auth_string.as_bytes());

        client
            .get(&url)
            .header("Authorization", format!("Basic {}", encoded_auth))
            .header("Accept", "application/json")
            .send()
            .await?
    } else {
        response
    };

    if !response.status().is_success() {
        return Err(format!("Failed to fetch JIRA issue {}: {}", jira_key, response.status()).into());
    }

    let issue: JiraIssue = response.json().await?;
    Ok(issue)
}

fn format_outage_entry(
    record: &OutageRecord,
    start_time: Option<String>,
    end_time: Option<String>,
) -> String {
    let date = parse_date(&record.date)
        .map(|d| d.format("%B %d").to_string())
        .unwrap_or_else(|| record.date.clone());

    let time_range = match (start_time, end_time) {
        (Some(start), Some(end)) => {
            format!(" ({} - {} - {}min)", start, end, record.duration)
        }
        _ => {
            if !record.duration.is_empty() && record.duration != "0" {
                format!(" ({}min)", record.duration)
            } else {
                String::new()
            }
        }
    };

    let severity = if !record.severity.is_empty() {
        format!(" ({})", record.severity)
    } else {
        String::new()
    };

    format!(
        "{}{} {}{}\n{}",
        date,
        time_range,
        record.service,
        severity,
        format_description(&record.cause, &record.solution)
    )
}

fn format_description(cause: &str, solution: &str) -> String {
    let mut description = String::new();

    if !cause.is_empty() {
        description.push_str(cause);
        if !cause.ends_with('.') {
            description.push('.');
        }
    }

    if !solution.is_empty() {
        if !description.is_empty() {
            description.push_str(" ");
        }
        description.push_str(solution);
        if !solution.ends_with('.') {
            description.push('.');
        }
    }

    description
}

#[derive(Debug, Serialize)]
struct LMStudioRequest {
    model: String,
    messages: Vec<LMStudioMessage>,
    temperature: f32,
    max_tokens: i32,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct LMStudioMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct LMStudioResponse {
    choices: Vec<LMStudioChoice>,
}

#[derive(Debug, Deserialize)]
struct LMStudioChoice {
    message: LMStudioMessageResponse,
}

#[derive(Debug, Deserialize)]
struct LMStudioMessageResponse {
    content: String,
}

fn extract_time_from_description(description: &str) -> (Option<String>, Option<String>) {
    // Try to extract time range from description like "18:40 - 18:43"
    if description.contains('-') && description.contains(':') {
        if let Ok(time_pattern) = regex::Regex::new(r"(\d{1,2}:\d{2})\s*[-–]\s*(\d{1,2}:\d{2})") {
            if let Some(captures) = time_pattern.captures(description) {
                return (
                    captures.get(1).map(|m| m.as_str().to_string()),
                    captures.get(2).map(|m| m.as_str().to_string()),
                );
            }
        }
    }
    (None, None)
}

fn calculate_incident_times(date: &NaiveDate, duration_str: &str, jira_description: &str) -> (String, String) {
    // First try to extract times from JIRA description
    let (jira_start, jira_end) = extract_time_from_description(jira_description);
    if jira_start.is_some() && jira_end.is_some() {
        return (jira_start.unwrap(), jira_end.unwrap());
    }

    // Parse duration to get minutes
    let duration_minutes = parse_duration_to_minutes(duration_str);

    // If no specific times found, use reasonable business hour assumptions
    // Most incidents occur during business hours (09:00-17:00 UTC)
    let default_start_time = match date.weekday() {
        Weekday::Sat | Weekday::Sun => NaiveTime::from_hms_opt(14, 0, 0).unwrap(), // Weekend incidents often mid-day
        _ => match duration_minutes {
            0..=30 => NaiveTime::from_hms_opt(10, 30, 0).unwrap(), // Short incidents in morning
            31..=120 => NaiveTime::from_hms_opt(13, 0, 0).unwrap(), // Medium incidents during midday
            _ => NaiveTime::from_hms_opt(9, 0, 0).unwrap(), // Long incidents start early
        }
    };

    let start_datetime = NaiveDateTime::new(*date, default_start_time);
    let end_datetime = start_datetime + Duration::minutes(duration_minutes as i64);

    (
        start_datetime.format("%H:%M").to_string(),
        end_datetime.format("%H:%M").to_string(),
    )
}

fn parse_duration_to_minutes(duration_str: &str) -> i32 {
    let duration_str = duration_str.to_lowercase();

    // Handle various formats like "6", "600", "4+29", "4h29m", etc.
    if duration_str.contains("+") {
        // Handle "4+29" format - assume it's hours+minutes
        let parts: Vec<&str> = duration_str.split("+").collect();
        if parts.len() == 2 {
            let hours = parts[0].parse::<i32>().unwrap_or(0);
            let minutes = parts[1].replace("minutes", "").replace("min", "").trim().parse::<i32>().unwrap_or(0);
            return hours * 60 + minutes;
        }
    }

    if duration_str.contains("h") && duration_str.contains("m") {
        // Handle "4h29m" format
        if let Ok(pattern) = regex::Regex::new(r"(\d+)h(\d+)m") {
            if let Some(captures) = pattern.captures(&duration_str) {
                let hours = captures.get(1).unwrap().as_str().parse::<i32>().unwrap_or(0);
                let minutes = captures.get(2).unwrap().as_str().parse::<i32>().unwrap_or(0);
                return hours * 60 + minutes;
            }
        }
    }

    // Extract just the number and assume it's minutes
    if let Ok(pattern) = regex::Regex::new(r"(\d+)") {
        if let Some(captures) = pattern.captures(&duration_str) {
            return captures.get(1).unwrap().as_str().parse::<i32>().unwrap_or(5);
        }
    }

    5 // Default to 5 minutes if parsing fails
}

fn extract_rca_and_preventative_measures(description: &str) -> String {
    debug!("Attempting to extract RCA and Preventative Measures from description (length: {} chars)", description.len());
    let mut extracted_content = Vec::new();

    // Look for RCA section
    if let Ok(rca_pattern) = regex::Regex::new(r"(?i)(?:^|\n)\s*(?:RCA|Root Cause Analysis?)\s*:?\s*\n?(.*?)(?=\n\s*(?:[A-Z][^:\n]*:|$))") {
        if let Some(rca_match) = rca_pattern.captures(description) {
            if let Some(rca_content) = rca_match.get(1) {
                let cleaned_rca = rca_content.as_str().trim();
                if !cleaned_rca.is_empty() {
                    debug!("Found RCA section: {}", cleaned_rca);
                    extracted_content.push(format!("RCA: {}", cleaned_rca));
                } else {
                    debug!("Found RCA section but content was empty after trimming");
                }
            }
        } else {
            debug!("RCA pattern did not match");
        }
    }

    // Look for Preventative Measures section
    if let Ok(pm_pattern) = regex::Regex::new(r"(?i)(?:^|\n)\s*(?:Preventative Measures?|Prevention|Preventive Measures?)\s*:?\s*\n?(.*?)(?=\n\s*(?:[A-Z][^:\n]*:|$))") {
        if let Some(pm_match) = pm_pattern.captures(description) {
            if let Some(pm_content) = pm_match.get(1) {
                let cleaned_pm = pm_content.as_str().trim();
                if !cleaned_pm.is_empty() {
                    debug!("Found Preventative Measures section: {}", cleaned_pm);
                    extracted_content.push(format!("Preventative Measures: {}", cleaned_pm));
                } else {
                    debug!("Found Preventative Measures section but content was empty after trimming");
                }
            }
        } else {
            debug!("Preventative Measures pattern did not match");
        }
    }

    // If no specific sections found, look for any content that might be RCA-related
    if extracted_content.is_empty() {
        debug!("No RCA or Preventative Measures sections found, looking for fallback keywords");
        // Look for lines that might contain root cause information
        for line in description.lines() {
            let line = line.trim();
            if line.to_lowercase().contains("root cause") ||
               line.to_lowercase().contains("caused by") ||
               line.to_lowercase().contains("due to") {
                debug!("Found fallback RCA line: {}", line);
                extracted_content.push(line.to_string());
                break; // Just take the first relevant line to keep it concise
            }
        }
    }

    let result = extracted_content.join("\n");
    if result.is_empty() {
        debug!("No RCA or Preventative Measures content extracted");
    } else {
        debug!("Extracted content: {}", result);
    }

    result
}

async fn call_lm_studio(
    outages: &[OutageRecord],
    jira_details: &HashMap<String, JiraIssue>,
    week_number: u32,
    week_start: &NaiveDate,
    week_end: &NaiveDate,
    lm_studio_url: &str,
    model: &str,
) -> Result<String, Box<dyn Error>> {
    let mut outage_summaries = Vec::new();

    for record in outages {
        let jira_desc = if let Some(jira_key) = extract_jira_key(&record.ticket) {
            debug!("Processing JIRA ticket: {}", jira_key);
            if let Some(issue) = jira_details.get(&jira_key) {
                if let Some(desc) = issue.fields.description.as_ref() {
                    debug!("JIRA description found for {}, extracting RCA/PM", jira_key);
                    extract_rca_and_preventative_measures(desc)
                } else {
                    debug!("No description field found for {}", jira_key);
                    String::new()
                }
            } else {
                debug!("JIRA details not found in cache for {}", jira_key);
                String::new()
            }
        } else {
            debug!("Could not extract JIRA key from ticket: {}", record.ticket);
            String::new()
        };

        // Calculate start and end times
        let incident_date = parse_date(&record.date).unwrap_or_else(|| Local::now().date_naive());
        let full_jira_desc = if let Some(jira_key) = extract_jira_key(&record.ticket) {
            jira_details.get(&jira_key)
                .and_then(|issue| issue.fields.description.as_ref())
                .map(|d| d.clone())
                .unwrap_or_default()
        } else {
            String::new()
        };
        let (start_time, end_time) = calculate_incident_times(&incident_date, &record.duration, &full_jira_desc);

        let summary = format!(
            "Date: {}\nService: {}\nStart Time: {} UTC\nEnd Time: {} UTC\nDuration: {} minutes\nSeverity: {}\nCause: {}\nSolution: {}\nJIRA RCA/Preventative Measures: {}\n",
            record.date, record.service, start_time, end_time, record.duration, record.severity,
            record.cause, record.solution,
            if jira_desc.is_empty() { "N/A".to_string() } else { jira_desc }
        );
        outage_summaries.push(summary);
    }

    let prompt = format!(
        r#"Create a concise weekly stability report for week {} ({} to {}).

Format EXACTLY like these examples:

Sept 15th (18:40 - 18:43 - 3min) Sales-I DE API (Regional)
A configuration change by Microsoft Azure caused a temporary disruption to the CDN. As this originated from Azure's platform team, it was outside of our control. No action is required on our side, and service has since stabilized.

Sept 17th (15:07 - 15:12 - 5min) Sugar Market Mail App API (Regional)
The IIS logs initially pointed to a configuration issue, but further review with the Market team confirmed the root cause is a bug in the MsgApp application, which surfaced as database connectivity symptoms. Because MsgApp's current logging is insufficient to isolate the cause, Rachel from Market has opened a ticket to add more actionable logging. In parallel, the Market team is already migrating functionality from MsgApp to Vulcan with the goal of fully retiring MsgApp once the transition is complete.

Raw data:
{}

CRITICAL REQUIREMENTS:
- Each incident MUST clearly explain what we're doing to PREVENT it from happening again
- If preventative measures aren't clear from the data, mention what should be done in the AI Recommendations section
- Keep descriptions to 2-3 sentences maximum
- Use the provided Start Time and End Time in UTC format (HH:MM - HH:MM)
- Use Month day format (Sept 15th, not September 15)
- Format: Sept 15th (18:40 - 18:43 - 3min) Service Name (Severity)
- Combine root cause, immediate resolution, AND prevention steps
- Include severity in parentheses if available
- End the email portion with "Regards,"
- This will be read by the CEO and CTO

AFTER the email content, add a separate section titled "--- AI RECOMMENDATIONS ---" with any additional prevention suggestions you think would be beneficial that weren't mentioned in the incidents.
"#,
        week_number,
        week_start.format("%B %d"),
        week_end.format("%B %d"),
        outage_summaries.join("\n---\n")
    );

    let request = LMStudioRequest {
        model: model.to_string(),
        messages: vec![
            LMStudioMessage {
                role: "system".to_string(),
                content: "You are a technical writer creating executive stability reports. Focus heavily on PREVENTION - each incident must clearly explain what we're doing to prevent recurrence. Be concise and direct. Include AI recommendations after the email.".to_string(),
            },
            LMStudioMessage {
                role: "user".to_string(),
                content: prompt.clone(),
            },
        ],
        temperature: 0.3,
        max_tokens: 4000,
        stream: false,
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()?;

    debug!("Sending request to LM Studio");
    debug!("LM Studio request prompt:\n{}", prompt);

    let response = client
        .post(lm_studio_url)
        .json(&request)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
        return Err(format!("LM Studio API error {}: {}", status, error_text).into());
    }

    debug!("Parsing LM Studio response");
    let response_text = response.text().await?;

    // Try to parse the JSON response
    let lm_response: LMStudioResponse = serde_json::from_str(&response_text)
        .map_err(|e| format!("Failed to parse LM Studio response: {}. Response: {}", e, &response_text[..response_text.len().min(500)]))?;

    if let Some(choice) = lm_response.choices.first() {
        Ok(choice.message.content.clone())
    } else {
        Err("No response from LM Studio".into())
    }
}

fn get_week_number(date: &NaiveDate) -> u32 {
    date.iso_week().week()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenv().ok();
    env_logger::init();

    let jira_token = env::var("JIRA_TOKEN")
        .expect("JIRA_TOKEN not found in environment variables");

    let jira_email = env::var("JIRA_EMAIL")
        .unwrap_or_else(|_| {
            info!("JIRA_EMAIL not set, using default");
            "automation@sugarcrm.com".to_string()
        });

    let lm_studio_url = env::var("LM_STUDIO_URL")
        .unwrap_or_else(|_| "http://localhost:1234/v1/chat/completions".to_string());

    let lm_studio_model = env::var("LM_STUDIO_MODEL")
        .unwrap_or_else(|_| "local-model".to_string());

    let (week_start, week_end) = get_previous_week_range();
    let week_number = get_week_number(&week_start);

    info!("Generating report for week {} ({} - {})",
             week_number,
             week_start.format("%B %d"),
             week_end.format("%B %d"));

    let file = File::open("outages.csv")?;
    let mut reader = Reader::from_reader(file);

    let mut outages: Vec<OutageRecord> = Vec::new();
    let mut jira_details: HashMap<String, JiraIssue> = HashMap::new();

    for result in reader.deserialize() {
        let record: OutageRecord = match result {
            Ok(r) => r,
            Err(e) => {
                warn!("Skipping invalid record: {}", e);
                continue;
            }
        };

        if let Some(date) = parse_date(&record.date) {
            if date >= week_start && date <= week_end {
                outages.push(record);
            }
        }
    }

    outages.sort_by(|a, b| {
        let date_a = parse_date(&a.date);
        let date_b = parse_date(&b.date);
        date_a.cmp(&date_b)
    });

    info!("Found {} outage(s)", outages.len());
    debug!("Fetching JIRA details...");

    let mut jira_fetch_failed = false;
    for record in &outages {
        if let Some(jira_key) = extract_jira_key(&record.ticket) {
            debug!("Found JIRA key: {}", jira_key);
            if !jira_details.contains_key(&jira_key) {
                debug!("Fetching JIRA details for {}", jira_key);
                match fetch_jira_details(&jira_key, &jira_email, &jira_token).await {
                    Ok(issue) => {
                        debug!("Successfully fetched JIRA details for {}", jira_key);
                        jira_details.insert(jira_key.clone(), issue);
                    }
                    Err(e) => {
                        warn!("Failed to fetch JIRA details for {}: {}", jira_key, e);
                        jira_fetch_failed = true;
                    }
                }
            } else {
                debug!("JIRA details already cached for {}", jira_key);
            }
        } else {
            debug!("No JIRA key found in ticket URL: {}", record.ticket);
        }
    }

    if jira_fetch_failed {
        warn!("Some JIRA tickets could not be fetched, using CSV data only");
    }

    // Try to use LM Studio to format the report if configured
    let use_ai = env::var("USE_AI").unwrap_or_else(|_| "true".to_string()) == "true";

    if use_ai {
        info!("Generating AI-formatted report...");
    }

    let ai_result = if use_ai {
        call_lm_studio(
            &outages,
            &jira_details,
            week_number,
            &week_start,
            &week_end,
            &lm_studio_url,
            &lm_studio_model,
        )
        .await
    } else {
        Err("AI generation disabled".into())
    };

    match ai_result {
        Ok(ai_report) => {
            println!("{}", "=".repeat(80));
            println!("WEEKLY STABILITY REPORT (AI-Generated)");
            println!("{}", "=".repeat(80));
            println!();
            println!("{}", ai_report);
        }
        Err(e) => {
            warn!("Could not generate AI report: {}", e);
            info!("Using standard format");

            // Fallback to original formatting
            println!("{}", "=".repeat(80));
            println!("WEEKLY STABILITY REPORT");
            println!("Week {} ({} - {})", week_number, week_start.format("%B %d"), week_end.format("%B %d"));
            println!("All times UTC");
            println!("{}", "=".repeat(80));
            println!();

            for record in &outages {
                let jira_key = extract_jira_key(&record.ticket);
                let (start_time, end_time) = if let Some(ref key) = jira_key {
                    if let Some(issue) = jira_details.get(key) {
                        if let Some(ref desc) = issue.fields.description {
                            extract_time_from_description(desc)
                        } else {
                            (None, None)
                        }
                    } else {
                        (None, None)
                    }
                } else {
                    (None, None)
                };

                let entry = format_outage_entry(&record, start_time, end_time);
                println!("{}\n", entry);
            }

            println!("Regards,");
        }
    }

    Ok(())
}
