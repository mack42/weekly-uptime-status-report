# Weekly Status Report Generator

This Rust application automatically generates weekly stability reports from outage data, with optional JIRA integration and AI-powered formatting via LM Studio.

## Features

- **CSV Data Processing**: Reads outage data from `outages.csv`
- **Automatic Week Filtering**: Automatically filters to previous week's data (Sunday to Saturday)
- **JIRA Integration**: Attempts to fetch additional details from JIRA tickets
- **AI-Powered Formatting**: Uses LM Studio to format reports in executive-friendly language
- **Fallback Support**: Works even if JIRA or LM Studio are unavailable

## Setup

### 1. Environment Variables

Create or update `.env` file with:

```bash
# JIRA Configuration
JIRA_TOKEN=your_jira_api_token
JIRA_EMAIL=your_email@company.com

# LM Studio Configuration (optional)
LM_STUDIO_URL=http://localhost:1234/v1/chat/completions
LM_STUDIO_MODEL=local-model
```

### 2. LM Studio Setup (Optional but Recommended)

1. Download and install [LM Studio](https://lmstudio.ai/)
2. Download a model (recommended: Llama 3, Mistral, or similar)
3. Start the local server in LM Studio:
   - Go to "Server" tab
   - Select your model
   - Click "Start Server"
   - Default port is 1234

### 3. CSV File Format

Ensure `outages.csv` has the following columns:
- Date (format: DD/Mon/YY, e.g., "28/Sep/25")
- Ticket (JIRA URL)
- Name
- CloudStack/Service
- Duration (in minutes)
- Cause
- Solution
- Assignee
- Status
- Severity

## Usage

```bash
# Build the application
cargo build --release

# Run the report generator
cargo run

# Or run the compiled binary
./target/release/weekly-status-report
```

## How It Works

1. **Data Collection**:
   - Reads outages from CSV file
   - Filters to previous week's data
   - Attempts to fetch JIRA ticket descriptions (if accessible)

2. **AI Processing** (if LM Studio is running):
   - Sends outage data to local LLM
   - Requests formatting in executive-appropriate language
   - Extracts time ranges from descriptions
   - Combines cause/solution into coherent narratives

3. **Output Generation**:
   - If LM Studio succeeds: Shows AI-formatted report
   - If LM Studio fails: Falls back to standard formatting
   - Always includes all outages from the previous week

## Sample Output

```
================================================================================
WEEKLY STABILITY REPORT (AI-Generated)
================================================================================

Week 38 (September 21 - September 27)
All times UTC

September 21st (15:30 - 15:36 - 6min) Sales-I US (S2)
US Production Sophos firewall unexpectedly restarted due to a known bug in the latest update causing memory leaks when multiple services run simultaneously. The issue originated from Sophos firmware and was outside our control. Service recovered automatically, with a full fix pending firmware update rollout.

September 23rd (10:15 - 10:20 - 5min) Sugar Market EU Services (S1)
Multiple Sugar Market services in the EU region experienced 5xx errors and timeouts when the Nginx service failed to restart on both euw1-msgapp-nginx-proxy nodes. The incident was resolved by manually restarting the Nginx service on the affected instances.

Regards,
```

## Troubleshooting

### JIRA Authentication Issues
- Ensure your API token is generated from Atlassian Account Settings
- Verify you have access to the OPS project
- Check that JIRA_EMAIL matches your Atlassian account email

### LM Studio Connection Failed
- Verify LM Studio server is running
- Check the port number (default: 1234)
- Ensure a model is loaded in LM Studio
- Test with: `curl http://localhost:1234/v1/models`

### CSV Parsing Errors
- Check date format (DD/Mon/YY)
- Ensure all required columns are present
- Remove any extra commas in data fields

## Dependencies

- `chrono`: Date/time handling
- `csv`: CSV parsing
- `reqwest`: HTTP requests for JIRA and LM Studio
- `serde`: JSON serialization
- `tokio`: Async runtime
- `regex`: Time extraction from text