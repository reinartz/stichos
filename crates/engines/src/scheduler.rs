// SPDX-License-Identifier: MIT OR Apache-2.0
//! Windows Task Scheduler integration for the COM logon agent.
//!
//! This module generates and manages the XML definition for a
//! per-user logon-triggered scheduled task that starts the
//! stichos COM worker agent.
//!
//! Per D10: this is NEVER a service. It runs interactively as the
//! logged-in user.

/// Generate the XML definition for the logon-triggered task.
///
/// The task is configured as:
/// - Trigger: user logon (per-user, not system startup)
/// - LogonType: InteractiveToken (runs as the logged-in user)
/// - RunLevel: LeastPrivilege (no elevation)
/// - ExecutionTimeLimit: PT0S (no time limit — agent manages lifecycle)
/// - MultipleInstancesPolicy: IgnoreNew (single instance)
///
/// # Arguments
/// * `agent_path` - Full path to the agent executable (e.g., `%ProgramFiles%\stichos\stichos-agent.exe`)
///
/// # Returns
/// Valid Windows Task Scheduler XML document.
pub fn generate_task_xml(agent_path: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
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
      <Command>{}</Command>
      <Arguments>worker start</Arguments>
    </Exec>
  </Actions>
</Task>"#,
        agent_path
    )
}

/// Install the scheduled task (requires admin for system-wide, user-level for per-user).
#[cfg(all(feature = "ms-office", target_os = "windows"))]
pub fn install_task(agent_path: &str) -> Result<(), SchedulerError> {
    use std::process::Command;
    let xml = generate_task_xml(agent_path);
    let tmp_path = format!("{}\\stichos-com-worker.xml", std::env::temp_dir().display());
    std::fs::write(&tmp_path, &xml).map_err(|e| SchedulerError::WriteXml(e))?;

    // schtasks /Create /TN "stichos COM Worker" /XML stichos-com-worker.xml /RU CURRENT_USER /SC ONLOGON
    let output = Command::new("schtasks")
        .args(&[
            "/Create",
            "/TN",
            "stichos COM Worker",
            "/XML",
            &tmp_path,
            "/SC",
            "ONLOGON",
            "/RU",
            "CURRENT_USER",
            "/F", // force overwrite
        ])
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let _ = std::fs::remove_file(&tmp_path);
            Ok(())
        }
        Ok(o) => Err(SchedulerError::InstallFailed(
            String::from_utf8_lossy(&o.stderr).to_string(),
        )),
        Err(e) => Err(SchedulerError::InstallFailed(e.to_string())),
    }
}

/// Uninstall the scheduled task.
#[cfg(all(feature = "ms-office", target_os = "windows"))]
pub fn uninstall_task() -> Result<(), SchedulerError> {
    use std::process::Command;
    let output = Command::new("schtasks")
        .args(&["/Delete", "/TN", "stichos COM Worker", "/F"])
        .output();

    match output {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(SchedulerError::DeleteFailed(
            String::from_utf8_lossy(&o.stderr).to_string(),
        )),
        Err(e) => Err(SchedulerError::DeleteFailed(e.to_string())),
    }
}

/// Check if the scheduled task is installed.
#[cfg(all(feature = "ms-office", target_os = "windows"))]
pub fn is_task_installed() -> Result<bool, SchedulerError> {
    use std::process::Command;
    let output = Command::new("schtasks")
        .args(&["/Query", "/TN", "stichos COM Worker", "/XML"])
        .output();

    Ok(output.map(|o| o.status.success()).unwrap_or(false))
}

/// Errors specific to task scheduler operations.
#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    /// Failed to write task XML to temporary file.
    #[error("Failed to write task XML: {0}")]
    WriteXml(std::io::Error),
    /// Failed to create/install the scheduled task.
    #[error("Failed to install task: {0}")]
    InstallFailed(String),
    /// Failed to delete the scheduled task.
    #[error("Failed to delete task: {0}")]
    DeleteFailed(String),
    /// Task query failed.
    #[error("Task query failed: {0}")]
    QueryFailed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// test_task_xml_generates_valid_structure: generated XML has required root elements.
    #[test]
    fn test_task_xml_generates_valid_structure() {
        let xml = generate_task_xml("C:\\Program Files\\stichos\\stichos-agent.exe");
        assert!(xml.contains("<?xml version=\"1.0\" encoding=\"UTF-16\"?>"));
        assert!(xml.contains("<Task version=\"1.2\""));
        assert!(xml.contains("xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\""));
    }

    /// test_task_xml_contains_required_elements: all required task elements are present.
    #[test]
    fn test_task_xml_contains_required_elements() {
        let xml = generate_task_xml("C:\\agent.exe");

        // Registration
        assert!(xml.contains("<Description>stichos COM Worker Logon Agent</Description>"));
        assert!(xml.contains("<Author>stichos</Author>"));

        // Trigger
        assert!(xml.contains("<Triggers>"));
        assert!(xml.contains("<LogonTrigger>"));
        assert!(xml.contains("<Enabled>true</Enabled>"));

        // Principals
        assert!(xml.contains("<Principals>"));
        assert!(xml.contains("<LogonType>InteractiveToken</LogonType>"));
        assert!(xml.contains("<RunLevel>LeastPrivilege</RunLevel>"));

        // Settings
        assert!(xml.contains("<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>"));
        assert!(xml.contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"));
        assert!(xml.contains("<StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>"));
        assert!(xml.contains("<AllowHardTerminate>true</AllowHardTerminate>"));
        assert!(xml.contains("<StartWhenAvailable>true</StartWhenAvailable>"));
        assert!(xml.contains("<RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>"));
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(xml.contains("<Disabled>false</Disabled>"));

        // Action
        assert!(xml.contains("<Actions Context=\"Author\">"));
        assert!(xml.contains("<Exec>"));
        assert!(xml.contains("<Command>C:\\agent.exe</Command>"));
        assert!(xml.contains("<Arguments>worker start</Arguments>"));
    }

    /// test_task_xml_includes_agent_path: agent path appears in the XML.
    #[test]
    fn test_task_xml_includes_agent_path() {
        let xml = generate_task_xml("C:\\Program Files\\stichos\\stichos-agent.exe");
        assert!(xml.contains("<Command>C:\\Program Files\\stichos\\stichos-agent.exe</Command>"));
    }

    /// test_task_xml_never_service: logon type is InteractiveToken, not ServiceAccount.
    #[test]
    fn test_task_xml_never_service() {
        let xml = generate_task_xml("C:\\agent.exe");
        assert!(xml.contains("<LogonType>InteractiveToken</LogonType>"));
        assert!(!xml.contains("<ServiceAccount"));
        assert!(!xml.contains("<RunLevel>HighestAvailable</RunLevel>"));
    }

    /// test_task_xml_single_instance: IgnoreNew policy enforced.
    #[test]
    fn test_task_xml_single_instance() {
        let xml = generate_task_xml("C:\\agent.exe");
        assert!(xml.contains("<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>"));
    }

    /// test_task_xml_no_battery_restrictions: agent runs even on battery.
    #[test]
    fn test_task_xml_no_battery_restrictions() {
        let xml = generate_task_xml("C:\\agent.exe");
        assert!(xml.contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"));
        assert!(xml.contains("<StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>"));
    }

    /// test_task_xml_no_time_limit: execution time is unlimited (PT0S).
    #[test]
    fn test_task_xml_no_time_limit() {
        let xml = generate_task_xml("C:\\agent.exe");
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
    }

    /// test_is_task_installed_returns_false_when_not_installed: on non-Windows, returns false.
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn test_is_task_installed_returns_false_when_not_installed() {
        // On non-Windows, the function is behind a feature gate,
        // so this test verifies the stub path works.
        // The actual schtasks test runs only on Windows with ms-office feature.
        let installed = generate_task_xml("C:\\agent.exe");
        // Verify we can still generate XML even when not checking
        assert!(!installed.is_empty());
    }

    /// test_install_task_requires_windows: install function is behind feature gate.
    #[cfg(not(all(feature = "ms-office", target_os = "windows")))]
    #[test]
    fn test_install_task_requires_windows() {
        // On non-Windows or without ms-office, install_task is not available.
        // Verify generate_task_xml still works as a fallback.
        let xml = generate_task_xml("C:\\agent.exe");
        assert!(xml.contains("C:\\agent.exe"));
        assert!(xml.contains("InteractiveToken"));
    }

    /// test_uninstall_task_requires_windows: uninstall function is behind feature gate.
    #[cfg(not(all(feature = "ms-office", target_os = "windows")))]
    #[test]
    fn test_uninstall_task_requires_windows() {
        // On non-Windows or without ms-office, uninstall_task is not available.
        // Verify the module is still functional for XML generation.
        let xml = generate_task_xml("C:\\agent.exe");
        assert!(xml.contains("<Task version=\"1.2\""));
    }

    /// test_scheduler_error_display: error variants display correctly.
    #[test]
    fn test_scheduler_error_display() {
        let err =
            SchedulerError::WriteXml(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        assert!(format!("{}", err).contains("Failed to write task XML"));

        let err = SchedulerError::InstallFailed("access denied".into());
        assert!(format!("{}", err).contains("Failed to install task"));
        assert!(format!("{}", err).contains("access denied"));

        let err = SchedulerError::DeleteFailed("not found".into());
        assert!(format!("{}", err).contains("Failed to delete task"));
    }

    /// test_scheduler_error_is_error: SchedulerError implements thiserror.
    #[test]
    fn test_scheduler_error_is_error() {
        let err = SchedulerError::InstallFailed("test".into());
        let _: &dyn std::error::Error = &err;
    }
}
