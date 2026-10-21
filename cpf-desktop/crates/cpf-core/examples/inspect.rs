use cpf_core::Workspace;

fn main() -> Result<(), String> {
    eprintln!("discovering");
    let workspace = Workspace::discover()?;
    eprintln!("listing");
    let machines = workspace.machines()?;
    for machine in machines {
        eprintln!("reading {}", machine.name);
        let detail = workspace.detail(&machine.id)?;
        let _settings = workspace.settings(&machine.id)?;
        println!(
            "{}: ready={}, menu_verified={}",
            machine.name,
            detail.readiness.ready_to_attempt_boot,
            detail.readiness.menu_boot_verified
        );
        if std::env::args().any(|arg| arg == "--verify") {
            let report = workspace.verify_preservation(&machine.id)?;
            println!(
                "{}: preservation_verified={}, checks={}",
                machine.name,
                report.all_verified,
                report.checks.len()
            );
            for check in report.checks.iter().filter(|check| !check.matches) {
                println!(
                    "  FAILED {}: expected {}, got {}",
                    check.label, check.expected, check.actual
                );
            }
        }
        if std::env::args().any(|arg| arg == "--compare-sd") {
            let comparison = workspace.compare_storage(&machine.id)?;
            println!(
                "{}: captured_sd_entries={}, current_sd_entries={:?}, contents_indexed={}",
                machine.name,
                comparison.historical_entry_count,
                comparison.current_entry_count,
                comparison.current_contents_indexed
            );
        }
        if std::env::args().any(|arg| arg == "--verify-sd") {
            for snapshot in workspace.sd_snapshots(&machine.id)? {
                if snapshot.schema == "cpf.dsi.sd-image-snapshot/1" {
                    match workspace.verify_sd_image_snapshot(&machine.id, &snapshot.snapshot_id) {
                        Ok(()) => println!(
                            "{}: SD snapshot {} verified",
                            machine.name, snapshot.snapshot_id
                        ),
                        Err(error) => println!(
                            "{}: SD snapshot {} FAILED: {}",
                            machine.name, snapshot.snapshot_id, error
                        ),
                    }
                }
            }
        }
    }
    Ok(())
}
