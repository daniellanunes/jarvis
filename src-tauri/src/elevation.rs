//! Admin rights asked once.
//!
//! Jarvis needs administrator rights (system actions, FPS through ETW). Asking
//! UAC at every start is what Windows does by default; instead, the installer
//! (elevated once) registers a scheduled task that runs Jarvis "with highest
//! privileges". Afterwards any normal start — Start menu, start with Windows —
//! just asks the Task Scheduler to run that task, and Jarvis comes up elevated
//! with no prompt. If the task is missing (portable copy, task deleted), the
//! old UAC prompt is the fallback, and that elevated run registers the task so
//! it is the last prompt.

#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::Path;

/// Task Scheduler path of the task.
pub const TASK: &str = r"\Jarvis\Launch";
/// Argument the task passes, so a start through it never asks for it again.
pub const FROM_TASK: &str = "--from-task";
const NO_WINDOW: u32 = 0x0800_0000;

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The task definition: on demand only (no trigger — starting with Windows is
/// the autostart entry's job, which runs this task), highest privileges, for
/// the signed-in user, no time limit, also on battery, parallel starts allowed
/// so a second start reaches the running instance (single-instance).
pub fn task_xml(exe: &Path, user: &str, highest: bool) -> String {
    let dir = exe.parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default();
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Abre o Jarvis com permissão de administrador sem perguntar de novo (criada na instalação).</Description>
    <URI>{task}</URI>
  </RegistrationInfo>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>{level}</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>Parallel</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>4</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>"{exe}"</Command>
      <Arguments>{arg}</Arguments>
      <WorkingDirectory>{dir}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#,
        task = xml_escape(TASK),
        user = xml_escape(user),
        level = if highest { "HighestAvailable" } else { "LeastPrivilege" },
        exe = xml_escape(&exe.to_string_lossy()),
        arg = FROM_TASK,
        dir = xml_escape(&dir),
    )
}

/// "DOMAIN\user" of the signed-in user.
pub fn current_user() -> String {
    let user = std::env::var("USERNAME").unwrap_or_default();
    match std::env::var("USERDOMAIN") {
        Ok(d) if !d.is_empty() => format!("{d}\\{user}"),
        _ => user,
    }
}

#[cfg(windows)]
fn schtasks(args: &[&str]) -> std::io::Result<std::process::Output> {
    std::process::Command::new("schtasks").args(args).creation_flags(NO_WINDOW).output()
}

/// The registered task's XML, if there is one.
#[cfg(windows)]
pub fn task_definition(name: &str) -> Option<String> {
    let out = schtasks(&["/query", "/tn", name, "/xml"]).ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

/// True when the task exists and starts this very executable.
#[cfg(windows)]
pub fn task_ready(exe: &Path) -> bool {
    task_definition(TASK).is_some_and(|x| x.to_lowercase().contains(&exe.to_string_lossy().to_lowercase()))
}

/// Starts Jarvis through the task (elevated, no prompt). False if it could not.
#[cfg(windows)]
pub fn run_task() -> bool {
    schtasks(&["/run", "/tn", TASK]).map(|o| o.status.success()).unwrap_or(false)
}

/// Registers (or replaces) the task under `name`. Needs admin for `highest`.
#[cfg(windows)]
pub fn register_as(name: &str, exe: &Path, highest: bool) -> Result<(), String> {
    let xml = task_xml(exe, &current_user(), highest).replace(TASK, name);
    let file = std::env::temp_dir().join(format!("jarvis-task-{}.xml", std::process::id()));
    // schtasks reads the XML as UTF-16 (with BOM), as it writes it.
    let mut bytes = vec![0xFF, 0xFE];
    for u in xml.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
    let out = schtasks(&["/create", "/tn", name, "/xml", &file.to_string_lossy(), "/f"]).map_err(|e| e.to_string());
    let _ = std::fs::remove_file(&file);
    let out = out?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("schtasks: {}{}", String::from_utf8_lossy(&out.stdout).trim(), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

#[cfg(windows)]
pub fn register(exe: &Path) -> Result<(), String> {
    register_as(TASK, exe, true)
}

#[cfg(windows)]
pub fn unregister() {
    let _ = schtasks(&["/delete", "/tn", TASK, "/f"]);
    // The folder the task lived in, when it is empty.
    let _ = schtasks(&["/delete", "/tn", r"\Jarvis", "/f"]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_task_runs_this_exe_elevated_with_its_marker() {
        let xml = task_xml(Path::new(r"C:\Program Files\Jarvis\openjarvis.exe"), r"PC\Ana & Co", true);
        assert!(xml.contains(r#"<Command>"C:\Program Files\Jarvis\openjarvis.exe"</Command>"#));
        assert!(xml.contains("<RunLevel>HighestAvailable</RunLevel>"));
        assert!(xml.contains(&format!("<Arguments>{FROM_TASK}</Arguments>")));
        assert!(xml.contains(r"<UserId>PC\Ana &amp; Co</UserId>"));
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(!xml.contains("<Triggers>"));
    }

    /// Creates and deletes a non-elevated copy, to prove schtasks accepts the
    /// XML (`cargo test task_xml_is_accepted -- --ignored`).
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn task_xml_is_accepted_by_windows() {
        let name = r"\Jarvis\XmlTest";
        let exe = std::env::current_exe().unwrap();
        register_as(name, &exe, false).unwrap();
        let def = task_definition(name).unwrap();
        let _ = schtasks(&["/delete", "/tn", name, "/f"]);
        let _ = schtasks(&["/delete", "/tn", r"\Jarvis", "/f"]);
        assert!(def.contains("--from-task"), "{def}");
    }
}

/// Installer, after copying the files (elevated): register the task and, on
/// the first install, turn "start with Windows" on.
#[cfg(windows)]
pub fn on_install() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut s = crate::settings::load();
    if !s.jarvis_install_defaults_applied {
        s.autostart = true;
        s.jarvis_install_defaults_applied = true;
        let _ = crate::settings::save(&s);
    }
    register(&exe)
}

/// Uninstaller, before removing the files: no task, no start with Windows.
#[cfg(windows)]
pub fn on_uninstall() {
    unregister();
    for key in [
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run",
    ] {
        for name in ["Jarvis", "OpenJarvis"] {
            let _ = std::process::Command::new("reg").args(["delete", key, "/v", name, "/f"]).creation_flags(NO_WINDOW).output();
        }
    }
}
