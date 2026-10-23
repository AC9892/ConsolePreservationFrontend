mod inspector;
mod link_audit;
use std::{path::PathBuf, sync::atomic::AtomicBool};
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let result = (|| -> Result<(), String> {
        if args.first().map(String::as_str) == Some("--link-audit") && args.len() == 3 {
            let report = link_audit::audit(&PathBuf::from(&args[1]), &PathBuf::from(&args[2]))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
            );
            return Ok(());
        }
        let root = args
            .first()
            .ok_or("Usage: PS4Inspector DUMP [--output REPORT.json]")?;
        let report = inspector::scan(&PathBuf::from(root), &AtomicBool::new(false), |_, _| {})?;
        if args.len() == 3 && args[1] == "--output" {
            inspector::export(&report, &PathBuf::from(&args[2]))?;
        } else if args.len() == 1 {
            println!(
                "{}",
                serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
            );
        } else {
            return Err("Usage: PS4Inspector DUMP [--output REPORT.json]".into());
        }
        if !report.complete {
            return Err("Partial report; see diagnostics".into());
        }
        Ok(())
    })();
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1)
    }
}
