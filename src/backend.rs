#[derive(Clone, Copy)]
pub struct BackendProfile {
    pub id: &'static str,
    pub backing_bytes: u64,
    pub headroom_bytes: u64,
}
pub fn profile_for_hash(hash: &str) -> Option<BackendProfile> {
    match hash {
        "d2844a3ad350e2c51edc6e61dc6684b29dcde76e23746f1bdf97cb9367ec88c3" => {
            Some(BackendProfile {
                id: "shadps4-v0.20.0",
                backing_bytes: 8448 * 1024 * 1024,
                headroom_bytes: 512 * 1024 * 1024,
            })
        }
        "c94348de9710e91dd8067920915dd6d1915b0cda6e5da9e4748b01b9cc396368" => {
            Some(BackendProfile {
                id: "shadps4-v0.11.0-comparison",
                backing_bytes: 5888 * 1024 * 1024,
                headroom_bytes: 128 * 1024 * 1024,
            })
        }
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_exact_verified_hashes_select_a_profile() {
        assert!(profile_for_hash("").is_none());
        assert!(profile_for_hash(
            "d2844a3ad350e2c51edc6e61dc6684b29dcde76e23746f1bdf97cb9367ec88c4"
        )
        .is_none());
        let comparison =
            profile_for_hash("c94348de9710e91dd8067920915dd6d1915b0cda6e5da9e4748b01b9cc396368")
                .unwrap();
        assert_eq!(comparison.id, "shadps4-v0.11.0-comparison");
        assert_eq!(comparison.backing_bytes, 5888 * 1024 * 1024);
    }
}
