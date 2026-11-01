use std::process::ExitCode;

use playkit_updater::util::ErrorKind;

fn main() -> ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    match playkit_updater::cli::run(args) {
        Ok(code) => ExitCode::from(code as u8),
        Err(error) => {
            eprintln!("PlayKit 更新失败：{}", error.user);
            eprintln!("技术详情：{}", error.detail);
            let code = match error.kind {
                ErrorKind::Blocked => 2,
                ErrorKind::Failed => 1,
                ErrorKind::Usage => 64,
                ErrorKind::Internal => 70,
            };
            ExitCode::from(code)
        }
    }
}
