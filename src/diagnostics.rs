use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct RuntimeDiagnostics {
    pub ipmi_transport_stub_calls: usize,
    pub invalid_ipmi_clients: BTreeMap<String, usize>,
    pub last_critical_message: Option<String>,
    pub morpheus_client_initialization_failed: bool,
}
pub fn analyze(log: &str) -> RuntimeDiagnostics {
    let mut result = RuntimeDiagnostics::default();
    let client = regex::Regex::new(
        r"sceIpmiMgrCreateClient, pServiceName\[([^\]\r\n]{1,128})\].*m_clientKid=0x([0-9a-fA-F]+)",
    )
    .unwrap();
    for line in log.lines() {
        if line.contains("Stub: ipmimgr_call (nid: Hk7iHmGxB18)") {
            result.ipmi_transport_stub_calls += 1;
        }
        if let Some(fields) = client.captures(line) {
            if u64::from_str_radix(&fields[2], 16).ok() == Some(u32::MAX as u64) {
                *result
                    .invalid_ipmi_clients
                    .entry(fields[1].into())
                    .or_default() += 1;
            }
        }
        if line.contains("[SceMorpheusUpdClient] Initialize: create client failed:") {
            result.morpheus_client_initialization_failed = true;
        }
        if line.contains("<Critical>") {
            result.last_critical_message = Some(line.into());
        }
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distinguishes_invalid_clients_from_success_and_unrelated_errors() {
        let log = "[Core] <Error> Stub: ipmimgr_call (nid: Hk7iHmGxB18) called, returning zero\n\
            [Tty] sceIpmiMgrCreateClient, pServiceName[SceDbPreparationServer], pClient=123, m_clientKid=0xFFFFFFFF\n\
            [Tty] sceIpmiMgrCreateClient, pServiceName[SceDbPreparationServer], pClient=123, m_clientKid=0x4\n\
            [Debug] <Critical> guest exception\n";
        let result = analyze(log);
        assert_eq!(result.ipmi_transport_stub_calls, 1);
        assert_eq!(result.invalid_ipmi_clients["SceDbPreparationServer"], 1);
        assert!(!result.morpheus_client_initialization_failed);
        assert_eq!(
            result.last_critical_message.as_deref(),
            Some("[Debug] <Critical> guest exception")
        );
        assert!(analyze("host allocation failed")
            .invalid_ipmi_clients
            .is_empty());
    }
}
