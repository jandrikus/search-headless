use std::io::Write;

use crate::types::CommandStatus;

/// `partial` still carries usable results, so it exits 0.
pub fn exit_code_for(status: CommandStatus) -> i32 {
    match status {
        CommandStatus::Ok | CommandStatus::Partial => 0,
        _ => 1,
    }
}

/// Write the result and wait for it to land.
///
/// Exiting right after a write can drop the tail of a large payload when stdout
/// is a pipe, which is exactly how an agent calls this.
pub fn emit(text: &str) {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    let _ = handle.write_all(text.as_bytes());
    let _ = handle.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_usable_results_exit_zero() {
        assert_eq!(exit_code_for(CommandStatus::Ok), 0);
        assert_eq!(exit_code_for(CommandStatus::Partial), 0);
        assert_eq!(exit_code_for(CommandStatus::Blocked), 1);
        assert_eq!(exit_code_for(CommandStatus::Timeout), 1);
        assert_eq!(exit_code_for(CommandStatus::Error), 1);
    }
}
