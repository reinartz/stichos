// SPDX-License-Identifier: MIT OR Apache-2.0
//! MSI component descriptor for the COM logon agent.
//!
//! This module provides the data structures and build instructions
//! for creating an MSI installer package. On non-Windows or without
//! the `ms-office` feature, all functions return instructional stubs.
//!
//! The MSI is built externally using `msibuild` tooling during the
//! release process. This module only provides the manifest/data.

use serde::{Deserialize, Serialize};

/// MSI package metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsiPackage {
    /// Product name.
    pub product_name: String,
    /// Product version (semver).
    pub product_version: String,
    /// Manufacturer.
    pub manufacturer: String,
    /// Product GUID (for MSI).
    pub product_code: GuidWrapper,
    /// Upgrade GUID (for upgrades).
    pub upgrade_code: GuidWrapper,
    /// Languages.
    pub languages: Vec<String>,
    /// Minimum Windows version supported.
    pub min_windows_version: String,
}

impl Default for MsiPackage {
    fn default() -> Self {
        Self {
            product_name: "stichos COM Worker".into(),
            product_version: "0.1.0".into(),
            manufacturer: "stichos".into(),
            product_code: GuidWrapper::default(),
            upgrade_code: GuidWrapper::default(),
            languages: vec!["1033".into()], // English
            min_windows_version: "Windows 10 1903+".into(),
        }
    }
}

/// Wrapper for MSI GUID values.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuidWrapper(pub String);

/// MSI component: the logon agent executable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsiComponent {
    /// Component GUID.
    pub component_code: GuidWrapper,
    /// Feature the component belongs to.
    pub feature: String,
    /// Files to install.
    pub files: Vec<MsiFile>,
    /// Registry entries.
    pub registry: Vec<MsiRegistryEntry>,
    /// Scheduled task definition.
    pub scheduled_task: Option<MsiScheduledTask>,
}

/// A file to be installed by MSI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsiFile {
    /// Source name (in the MSI cabinet).
    pub source_name: String,
    /// Destination folder.
    pub destination_name: String,
    /// Component owning the file.
    pub component: String,
    /// Key path (whether this file is the key path for the component).
    pub key_path: bool,
}

/// Registry entry for MSI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsiRegistryEntry {
    /// Registry key path.
    pub key: String,
    /// Value name.
    pub name: String,
    /// Value data.
    pub value: String,
    /// Value type: string, integer, etc.
    #[serde(rename = "type_")]
    pub type_: String,
}

/// Windows Task Scheduler XML definition (embedded in MSI).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsiScheduledTask {
    /// Task name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Trigger type (logon, startup, etc.).
    pub trigger: ScheduledTrigger,
    /// Action to perform.
    pub action: ScheduledAction,
    /// Run only if user is logged on (never a service).
    pub run_only_if_logged_on: bool,
    /// Disallow start if on batteries (laptop).
    pub disallow_start_if_on_batteries: bool,
    /// Stop if battery mode begins.
    pub stop_if_starts_on_batteries: bool,
}

/// Scheduled task trigger configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTrigger {
    /// Trigger type: "logon", "startup", etc.
    pub r#type: String,
    /// Current user if blank = all users.
    pub user: Option<String>,
}

/// Scheduled task action configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledAction {
    /// Action type: "exec".
    pub r#type: String,
    /// Path to the executable.
    pub path: String,
    /// Command-line arguments.
    pub arguments: Option<String>,
}

impl Default for ScheduledTrigger {
    fn default() -> Self {
        Self {
            r#type: "logon".into(),
            user: None,
        }
    }
}

impl Default for ScheduledAction {
    fn default() -> Self {
        Self {
            r#type: "exec".into(),
            path: String::new(),
            arguments: None,
        }
    }
}

/// Build the default MSI component for the COM logon agent.
///
/// This creates a component descriptor with:
/// - Agent binary at `%ProgramFiles%\stichos\stichos-agent.exe`
/// - Registry entry for the scheduled task
/// - Logon-triggered scheduled task definition
///
/// Per D10: the task runs as the logged-in user with InteractiveToken,
/// never as a system service.
pub fn build_logon_agent_component() -> MsiComponent {
    MsiComponent {
        component_code: GuidWrapper(
            "a1b2c3d4-e5f6-7890-abcd-ef1234567890".into(),
        ),
        feature: "LogonAgent".into(),
        files: vec![
            MsiFile {
                source_name: "stichos-agent.exe".into(),
                destination_name: "stichos-agent.exe".into(),
                component: "LogonAgentComponent".into(),
                key_path: true,
            },
        ],
        registry: vec![
            MsiRegistryEntry {
                key: "Software\\stichos\\LogonAgent".into(),
                name: "AgentPath".into(),
                value: "[%ProgramFiles]%\\stichos\\stichos-agent.exe".into(),
                type_: "string".into(),
            },
            MsiRegistryEntry {
                key: "Software\\stichos\\LogonAgent".into(),
                name: "TaskName".into(),
                value: "stichos COM Worker".into(),
                type_: "string".into(),
            },
        ],
        scheduled_task: Some(MsiScheduledTask {
            name: "stichos COM Worker".into(),
            description: "stichos COM Worker Logon Agent - prepares COM environment and registers workers at user logon".into(),
            trigger: ScheduledTrigger::default(),
            action: ScheduledAction {
                r#type: "exec".into(),
                path: "%ProgramFiles%\\stichos\\stichos-agent.exe".into(),
                arguments: Some("worker start".into()),
            },
            run_only_if_logged_on: true,
            disallow_start_if_on_batteries: false,
            stop_if_starts_on_batteries: false,
        }),
    }
}

/// Validate MSI component configuration for common errors.
///
/// Returns a list of validation warnings (non-fatal) and errors (fatal).
/// Used during build to catch configuration issues early.
pub fn validate_component(component: &MsiComponent) -> (Vec<String>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut errors = Vec::new();

    if component.component_code.0.is_empty() {
        errors.push("Component GUID is empty".into());
    }

    if component.files.is_empty() {
        errors.push("Component has no files".into());
    }

    for file in &component.files {
        if file.source_name.is_empty() {
            errors.push(format!(
                "File has empty source_name: {}",
                file.destination_name
            ));
        }
        if !file.key_path {
            warnings.push(format!(
                "File {} does not have key_path set",
                file.destination_name
            ));
        }
    }

    if let Some(task) = &component.scheduled_task {
        if task.trigger.r#type != "logon" {
            warnings.push(format!(
                "Trigger type is '{}', expected 'logon' for per-user logon agent",
                task.trigger.r#type
            ));
        }
        if task.action.path.is_empty() {
            errors.push("Scheduled task has empty action path".into());
        }
    }

    (warnings, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// test_msi_package_defaults: default package has correct values.
    #[test]
    fn test_msi_package_defaults() {
        let pkg = MsiPackage::default();
        assert_eq!(pkg.product_name, "stichos COM Worker");
        assert_eq!(pkg.product_version, "0.1.0");
        assert_eq!(pkg.manufacturer, "stichos");
        assert_eq!(pkg.languages, vec!["1033"]);
        assert_eq!(pkg.min_windows_version, "Windows 10 1903+");
    }

    /// test_msi_package_serializes: package round-trips through JSON.
    #[test]
    fn test_msi_package_serializes() {
        let pkg = MsiPackage::default();
        let json = serde_json::to_string(&pkg).unwrap();
        let back: MsiPackage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.product_name, pkg.product_name);
        assert_eq!(back.product_version, pkg.product_version);
        assert_eq!(back.manufacturer, pkg.manufacturer);
    }

    /// test_build_logon_agent_component: component builds with expected defaults.
    #[test]
    fn test_build_logon_agent_component() {
        let comp = build_logon_agent_component();
        assert_eq!(comp.feature, "LogonAgent");
        assert_eq!(comp.files.len(), 1);
        assert_eq!(comp.files[0].source_name, "stichos-agent.exe");
        assert!(comp.files[0].key_path);
        assert_eq!(comp.registry.len(), 2);
        assert!(comp.scheduled_task.is_some());

        let task = comp.scheduled_task.unwrap();
        assert_eq!(task.name, "stichos COM Worker");
        assert!(task.description.contains("COM Worker"));
        assert_eq!(task.trigger.r#type, "logon");
        assert_eq!(task.trigger.user, None);
        assert!(task.run_only_if_logged_on);
        assert!(!task.disallow_start_if_on_batteries);
        assert!(!task.stop_if_starts_on_batteries);
        assert_eq!(task.action.r#type, "exec");
        assert!(task.action.path.contains("stichos-agent.exe"));
        assert_eq!(task.action.arguments, Some("worker start".into()));
    }

    /// test_validate_component_clean: valid component produces no errors.
    #[test]
    fn test_validate_component_clean() {
        let comp = build_logon_agent_component();
        let (_warnings, errors) = validate_component(&comp);
        assert!(errors.is_empty());
    }

    /// test_validate_component_empty_guid: empty GUID is an error.
    #[test]
    fn test_validate_component_empty_guid() {
        let comp = build_logon_agent_component();
        let mut bad = comp.clone();
        bad.component_code = GuidWrapper::default();
        let (_warnings, errors) = validate_component(&bad);
        assert!(errors.iter().any(|e| e.contains("GUID")));
    }

    /// test_validate_component_no_files: no files is an error.
    #[test]
    fn test_validate_component_no_files() {
        let mut comp = build_logon_agent_component();
        comp.files.clear();
        let (_warnings, errors) = validate_component(&comp);
        assert!(errors.iter().any(|e| e.contains("no files")));
    }

    /// test_validate_component_empty_task_path: empty task path is an error.
    #[test]
    fn test_validate_component_empty_task_path() {
        let mut comp = build_logon_agent_component();
        if let Some(ref mut task) = comp.scheduled_task {
            task.action.path.clear();
        }
        let (_warnings, errors) = validate_component(&comp);
        assert!(errors.iter().any(|e| e.contains("empty action path")));
    }

    /// test_scheduled_trigger_defaults: defaults to logon trigger.
    #[test]
    fn test_scheduled_trigger_defaults() {
        let trigger = ScheduledTrigger::default();
        assert_eq!(trigger.r#type, "logon");
        assert!(trigger.user.is_none());
    }

    /// test_guid_wrapper_default: default is empty string.
    #[test]
    fn test_guid_wrapper_default() {
        let guid = GuidWrapper::default();
        assert_eq!(guid.0, "");
    }

    /// test_msi_file_serializes: file round-trips through JSON.
    #[test]
    fn test_msi_file_serializes() {
        let file = MsiFile {
            source_name: "agent.exe".into(),
            destination_name: "agent.exe".into(),
            component: "LogonAgentComponent".into(),
            key_path: true,
        };
        let json = serde_json::to_string(&file).unwrap();
        let back: MsiFile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.source_name, file.source_name);
        assert_eq!(back.key_path, file.key_path);
    }

    /// test_msi_registry_entry_serializes: registry entry round-trips.
    #[test]
    fn test_msi_registry_entry_serializes() {
        let entry = MsiRegistryEntry {
            key: "HKCU\\Software\\stichos".into(),
            name: "Path".into(),
            value: "C:\\Program Files\\stichos".into(),
            type_: "string".into(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: MsiRegistryEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.key, entry.key);
        assert_eq!(back.type_, "string");
    }

    /// test_msi_scheduled_task_serializes: task round-trips through JSON.
    #[test]
    fn test_msi_scheduled_task_serializes() {
        let task = MsiScheduledTask {
            name: "Test Task".into(),
            description: "A test task".into(),
            trigger: ScheduledTrigger {
                r#type: "logon".into(),
                user: Some("user".into()),
            },
            action: ScheduledAction {
                r#type: "exec".into(),
                path: "C:\\agent.exe".into(),
                arguments: Some("--verbose".into()),
            },
            run_only_if_logged_on: true,
            disallow_start_if_on_batteries: true,
            stop_if_starts_on_batteries: true,
        };
        let json = serde_json::to_string(&task).unwrap();
        let back: MsiScheduledTask = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, task.name);
        assert_eq!(back.trigger.user, Some("user".into()));
        assert!(back.run_only_if_logged_on);
    }
}
