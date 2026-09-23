# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Logon Agent Rehearsal Script
#
# This script rehearses the full lifecycle of the COM logon agent
# scheduled task on Windows:
#   1. Generates the task XML from the scheduler module
#   2. Installs it via schtasks
#   3. Verifies installation
#   4. Triggers a test run
#   5. Cleans up (uninstalls)
#
# Usage: Run as the target user (not required admin for per-user task)
#   .\logon-agent-rehearsal.ps1

[CmdletBinding()]
param(
    [string]$AgentPath = (Join-Path $env:ProgramFiles "stichos\stichos-agent.exe"),
    [string]$TaskName = "stichos COM Worker"
)

$ErrorActionPreference = "Stop"
$ConfirmPreference = "None"

function Log-Info {
    param([string]$Message)
    Write-Host "[INFO] $Message" -ForegroundColor Cyan
}

function Log-Ok {
    param([string]$Message)
    Write-Host "[OK]   $Message" -ForegroundColor Green
}

function Log-Warn {
    param([string]$Message)
    Write-Host "[WARN] $Message" -ForegroundColor Yellow
}

function Log-Err {
    param([string]$Message)
    Write-Host "[ERR]  $Message" -ForegroundColor Red
}

# Generate the task XML definition
# On Windows with ms-office feature, this would call:
#   engines::scheduler::generate_task_xml(agent_path)
# Here we generate it inline for rehearsal.
Log-Info "Step 1/5: Generating task XML"

$xmlPath = Join-Path $env:TEMP "stichos-com-worker.xml"

$taskXml = @"
<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>stichos COM Worker Logon Agent</Description>
    <Author>stichos</Author>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="0">
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Disabled>false</Disabled>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>$AgentPath</Command>
      <Arguments>worker start</Arguments>
    </Exec>
  </Actions>
</Task>
"@

Set-Content -Path $xmlPath -Value $taskXml -Encoding UTF16
Log-Ok "Task XML generated at $xmlPath"

# Install the scheduled task
Log-Info "Step 2/5: Installing scheduled task"

try {
    $schtasksResult = schtasks /Create /TN $TaskName /XML $xmlPath /SC ONLOGON /RU CURRENT_USER /F
    Log-Ok "Task '$TaskName' installed successfully"
} catch {
    Log-Err "Failed to install task: $_"
    Remove-Item -Path $xmlPath -ErrorAction SilentlyContinue
    exit 1
}

# Verify installation
Log-Info "Step 3/5: Verifying installation"

try {
    $queryResult = schtasks /Query /TN $TaskName /V
    if ($queryResult -match $TaskName) {
        Log-Ok "Task verified: $TaskName is registered"
        $queryResult | ForEach-Object { Write-Host "  $_" }
    } else {
        Log-Warn "Task may not be registered correctly"
    }
} catch {
    Log-Err "Failed to verify task: $_"
}

# Trigger a test run (manual trigger)
Log-Info "Step 4/5: Triggering test run"

try {
    $triggerResult = schtasks /Run /TN $TaskName
    Log-Ok "Task triggered manually (will run at next logon by default)"

    # Check if task ran recently
    Start-Sleep -Seconds 2
    $statusResult = schtasks /Query /TN $TaskName /FO LIST /V 2>$null
    if ($statusResult) {
        $statusLines = $statusResult -split "`n" | Where-Object { $_ -match "Status|LastResult|LastRunTime" }
        $statusLines | ForEach-Object { Write-Host "  $_" }
    }
} catch {
    Log-Warn "Could not trigger task manually: $_"
    Log-Warn "Task will run at next user logon instead"
}

# Clean up (uninstall)
Log-Info "Step 5/5: Cleaning up (uninstalling task)"

try {
    $uninstallResult = schtasks /Delete /TN $TaskName /F
    Log-Ok "Task '$TaskName' uninstalled"
} catch {
    Log-Err "Failed to uninstall task: $_"
}

# Clean up XML file
if (Test-Path $xmlPath) {
    Remove-Item -Path $xmlPath -Force
    Log-Ok "Removed temporary XML at $xmlPath"
}

Log-Info "Rehearsal complete."
Log-Info ""
Log-Info "For production deployment:"
Log-Info "  1. Build MSI using external msibuild tooling (WiX/Orca)"
Log-Info "  2. Distribute MSI via GPO/Intune to target machines"
Log-Info "  3. Scheduled task runs 'stichos-agent.exe worker start' on each user logon"
Log-Info "  4. Task uses InteractiveToken + LeastPrivilege (never a service)"
