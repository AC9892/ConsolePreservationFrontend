//! Extract a narrow, path-free signal from a private Azahar log.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{self, Read},
    path::Path,
};

const MAX_LOG_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NsLaunchRequest {
    pub title_id: u64,
    pub flags: u32,
}

fn hex_word_after<'a>(line: &'a str, marker: &str) -> Option<(u32, &'a str)> {
    let (_, tail) = line.split_once(marker)?;
    let digits = tail
        .split_once([',', '}', ' '])
        .map(|(first, _)| first)
        .unwrap_or(tail);
    let value = u32::from_str_radix(digits, 16).ok()?;
    Some((value, tail))
}

pub fn parse_unimplemented_ns_launch(line: &str) -> Option<NsLaunchRequest> {
    if !line.contains("unknown / unimplemented function 'LaunchTitle': port='ns:s'")
        || !line.contains("[0]=0x200c0")
    {
        return None;
    }
    let (low, _) = hex_word_after(line, "[1]=0x")?;
    let (high, _) = hex_word_after(line, "[2]=0x")?;
    let (flags, _) = hex_word_after(line, "[3]=0x")?;
    Some(NsLaunchRequest {
        title_id: (u64::from(high) << 32) | u64::from(low),
        flags,
    })
}

pub struct NsLaunchReport {
    pub calls: usize,
    pub unique_titles: BTreeSet<u64>,
    pub module_titles: BTreeSet<u64>,
    pub other_titles: BTreeSet<u64>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct FailureReport {
    pub svc_results: BTreeMap<(u32, u32), usize>,
    pub fatal_calls: usize,
}

fn parse_failures(text: &str) -> FailureReport {
    let mut report = FailureReport::default();
    for line in text.lines() {
        if line.contains("Kernel.SVC <Error>") && line.contains("SVCWrapper") {
            if let (Some((svc, _)), Some((result, _))) = (
                hex_word_after(line, "svc=0x"),
                hex_word_after(line, "raw=0x"),
            ) {
                if svc <= 0xff && result != 0 {
                    *report.svc_results.entry((svc, result)).or_default() += 1;
                }
            }
        }
        if line.contains("Service.ERR <Critical>")
            && line.contains("ThrowFatalError:")
            && line.ends_with(": Fatal error")
        {
            report.fatal_calls += 1;
        }
    }
    report
}

pub fn read_failure_report(path: &Path) -> io::Result<FailureReport> {
    Ok(parse_failures(&read_bounded_log(path)?))
}

fn read_bounded_log(path: &Path) -> io::Result<String> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_LOG_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_LOG_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "runtime log exceeds scan limit",
        ));
    }
    String::from_utf8(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "runtime log is not UTF-8"))
}

pub fn read_ns_launch_report(path: &Path) -> io::Result<NsLaunchReport> {
    let text = read_bounded_log(path)?;
    let mut report = NsLaunchReport {
        calls: 0,
        unique_titles: BTreeSet::new(),
        module_titles: BTreeSet::new(),
        other_titles: BTreeSet::new(),
    };
    for line in text.lines() {
        if let Some(request) = parse_unimplemented_ns_launch(line) {
            report.calls += 1;
            report.unique_titles.insert(request.title_id);
            if request.title_id >> 32 == 0x0004_0130 {
                report.module_titles.insert(request.title_id);
            } else {
                report.other_titles.insert(request.title_id);
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_readers_reject_oversized_and_non_utf8_logs() {
        let path = std::env::temp_dir().join(format!("cpf-log-limits-{}", std::process::id()));
        File::create(&path)
            .unwrap()
            .set_len(MAX_LOG_BYTES + 1)
            .unwrap();
        assert!(read_failure_report(&path).is_err());
        assert!(read_ns_launch_report(&path).is_err());
        std::fs::write(&path, [0xff, 0xfe]).unwrap();
        assert!(read_failure_report(&path).is_err());
        assert!(read_ns_launch_report(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn failure_report_counts_exact_records_without_retaining_private_text() {
        let failure = "Kernel.SVC <Error> private/path:Kernel::SVCWrapper::Call:298: svc=0x25 raw=0xD8E007F7 level=27";
        let fatal = "Service.ERR <Critical> private/path:ERR_F::ThrowFatalError:169: Fatal error";
        let text = format!("{failure}\n{failure}\n{fatal}\nService.ERR <Critical> ThrowFatalError:171: Fatal error type: Generic\nKernel.SVC <Error> SVCWrapper svc=0x100 raw=0xD8E007F7\nKernel.SVC <Error> SVCWrapper svc=0x25 raw=0xNOTHEX\n");
        let report = parse_failures(&text);
        assert_eq!(report.fatal_calls, 1);
        assert_eq!(
            report.svc_results,
            BTreeMap::from([((0x25, 0xd8e007f7), 2)])
        );
        assert!(!format!("{report:?}").contains("private/path"));
        assert_eq!(parse_failures("unrelated text"), FailureReport::default());
    }

    #[test]
    fn only_exact_ns_launch_records_are_parsed() {
        let line = "Service <Error> unknown / unimplemented function 'LaunchTitle': port='ns:s' cmd_buf={[0]=0x200c0 (0x0002, 3, 0), [1]=0x1a02, [2]=0x40130, [3]=0x0}";
        assert_eq!(
            parse_unimplemented_ns_launch(line),
            Some(NsLaunchRequest {
                title_id: 0x0004_0130_0000_1a02,
                flags: 0
            })
        );
        assert_eq!(
            parse_unimplemented_ns_launch(&line.replace("port='ns:s'", "port='pm:app'")),
            None
        );
        assert_eq!(
            parse_unimplemented_ns_launch(&line.replace("[0]=0x200c0", "[0]=0x100c0")),
            None
        );
        assert_eq!(
            parse_unimplemented_ns_launch(&line.replace("[2]=0x40130", "[2]=oops")),
            None
        );
    }
}
