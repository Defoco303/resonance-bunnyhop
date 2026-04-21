use std::process::Command;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(target_os = "windows")]
fn hidden_command(program: &str) -> Command {
    let mut command = Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(not(target_os = "windows"))]
fn hidden_command(program: &str) -> Command {
    Command::new(program)
}

// Kept for future diagnostics surfaces even when the current UI doesn't render them.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ServiceDiagnostic {
    pub present: bool,
    pub running: bool,
}

// Kept for future diagnostics surfaces even when the current UI doesn't render them.
#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub struct EnvironmentDiagnostics {
    pub vigem_bus: ServiceDiagnostic,
    pub hidhide: ServiceDiagnostic,
    pub ds4windows_running: bool,
    pub dualsensex_running: bool,
}

pub fn collect_environment_diagnostics() -> EnvironmentDiagnostics {
    EnvironmentDiagnostics {
        vigem_bus: query_service("ViGEmBus"),
        hidhide: query_service("HidHide"),
        ds4windows_running: is_process_running("DS4Windows.exe"),
        dualsensex_running: is_process_running("DualSenseX.exe"),
    }
}

fn query_service(service_name: &str) -> ServiceDiagnostic {
    let Ok(output) = hidden_command("sc.exe")
        .args(["query", service_name])
        .output()
    else {
        return ServiceDiagnostic::default();
    };

    if !output.status.success() {
        return ServiceDiagnostic::default();
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_ascii_uppercase();
    ServiceDiagnostic {
        present: true,
        running: stdout.contains("RUNNING"),
    }
}

fn is_process_running(process_name: &str) -> bool {
    let Ok(output) = hidden_command("tasklist.exe")
        .args(["/FI", &format!("IMAGENAME eq {process_name}"), "/NH"])
        .output()
    else {
        return false;
    };

    let stdout = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
    stdout.contains(&process_name.to_ascii_lowercase())
}
