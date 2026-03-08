use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::utils::system_env::{ SystemOS};

pub struct Runtime {
    lima_instance: String,
}

impl Runtime {
    pub fn new() -> Self {
        let lima_instance =
            std::env::var("UPOD_LIMA_INSTANCE").unwrap_or_else(|_| "default".to_string());

        Self { lima_instance }
    }

    pub fn with_lima_instance(lima_instance: impl Into<String>) -> Self {
        Self {
            lima_instance: lima_instance.into(),
        }
    }

    pub fn create_container(&self, container_id: &str, bundle_dir: impl AsRef<Path>) -> Result<(), RuntimeError> {
        let bundle_dir = bundle_dir.as_ref();
        self.generate_spec(bundle_dir)?;
        self.exec_crun(
            &[
                "create".to_string(),
                "--bundle".to_string(),
                bundle_dir.display().to_string(),
                container_id.to_string(),
            ],
            SystemOS::current(),
        )
    }

    pub fn start_container(&self, container_id: &str) -> Result<(), RuntimeError> {
        self.exec_crun(&["start".to_string(), container_id.to_string()], SystemOS::current())
    }

    pub fn generate_spec(&self, bundle_dir: impl AsRef<Path>) -> Result<(), RuntimeError> {
        let bundle_dir = bundle_dir.as_ref();
        if !bundle_dir.exists() {
            return Err(RuntimeError::BundleNotFound(bundle_dir.to_path_buf()));
        }

        self.exec_crun(
            &[
                "spec".to_string(),
                "--bundle".to_string(),
                bundle_dir.display().to_string(),
            ],
            SystemOS::current(),
        )
    }

    fn exec_crun(&self, args: &[String], env: SystemOS) -> Result<(), RuntimeError> {
        let mut command = self.build_crun_command(args, env);
        let output = command.output()?;

        if output.status.success() {
            return Ok(());
        }

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

        Err(RuntimeError::CommandFailed {
            program: self.command_program(env).to_string(),
            args: self.command_args(args, env),
            status_code: output.status.code(),
            stdout,
            stderr,
        })
    }

    fn build_crun_command(&self, args: &[String], env: SystemOS) -> Command {
        let mut command = Command::new(self.command_program(env));
        command.args(self.command_args(args, env));
        command
    }

    fn command_program(&self, env: SystemOS) -> &str {
        match env {
            SystemOS::MacOS => "limactl",
            _ => "crun",
        }
    }

    fn command_args(&self, args: &[String], env: SystemOS) -> Vec<String> {
        match env {
            SystemOS::MacOS => {
                let mut full_args = Vec::with_capacity(4 + args.len());
                full_args.push("shell".to_string());
                full_args.push(self.lima_instance.clone());
                full_args.push("--".to_string());
                full_args.push("crun".to_string());
                full_args.extend(args.iter().cloned());
                full_args
            }
            _ => args.to_vec(),
        }
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub enum RuntimeError {
    BundleNotFound(PathBuf),
    CommandFailed {
        program: String,
        args: Vec<String>,
        status_code: Option<i32>,
        stdout: String,
        stderr: String,
    },
    Io(std::io::Error),
}

impl Display for RuntimeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            RuntimeError::BundleNotFound(path) => {
                write!(f, "bundle directory not found: {}", path.display())
            }
            RuntimeError::CommandFailed {
                program,
                args,
                status_code,
                stdout,
                stderr,
            } => {
                write!(
                    f,
                    "command failed: {} {} (status: {:?}, stdout: {}, stderr: {})",
                    program,
                    args.join(" "),
                    status_code,
                    stdout,
                    stderr
                )
            }
            RuntimeError::Io(err) => write!(f, "io error: {err}"),
        }
    }
}

impl std::error::Error for RuntimeError {}

impl From<std::io::Error> for RuntimeError {
    fn from(value: std::io::Error) -> Self {
        RuntimeError::Io(value)
    }
}

#[cfg(test)]
mod tests {
    use super::Runtime;
    use crate::utils::system_env::{SystemOS};

    #[test]
    fn build_linux_crun_command() {
        let runtime = Runtime::with_lima_instance("default");
        let args = vec![
            "create".to_string(),
            "--bundle".to_string(),
            "/tmp/demo".to_string(),
            "demo".to_string(),
        ];

        let command = runtime.build_crun_command(&args, SystemOS::Linux);

        assert_eq!(command.get_program().to_string_lossy(), "crun");
        let rendered_args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert_eq!(rendered_args, args);
    }

    #[test]
    fn build_macos_lima_crun_command() {
        let runtime = Runtime::with_lima_instance("upod-vm");
        let args = vec![
            "spec".to_string(),
            "--bundle".to_string(),
            "/tmp/demo".to_string(),
        ];

        let command = runtime.build_crun_command(&args, SystemOS::MacOS);

        assert_eq!(command.get_program().to_string_lossy(), "limactl");
        let rendered_args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            rendered_args,
            vec![
                "shell".to_string(),
                "upod-vm".to_string(),
                "--".to_string(),
                "crun".to_string(),
                "spec".to_string(),
                "--bundle".to_string(),
                "/tmp/demo".to_string()
            ]
        );
    }
}
