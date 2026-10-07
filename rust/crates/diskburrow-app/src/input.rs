//! UI-only confirmation state. A review identity must still match at commit.
#[derive(Default, Clone, Debug)]
pub struct ConfirmationGate {
    requested: Option<(String, bool)>,
}
impl ConfirmationGate {
    pub fn request(&mut self, id: &str, cleanup: bool, can_confirm: bool) -> bool {
        self.requested = can_confirm
            .then(|| (id.to_owned(), cleanup))
            .filter(|(id, _)| !id.is_empty());
        self.is_open()
    }
    pub fn take(&mut self, id: &str, cleanup: bool, can_confirm: bool) -> bool {
        self.requested
            .take()
            .is_some_and(|(review, mode)| can_confirm && review == id && mode == cleanup)
    }
    pub fn dismiss(&mut self) {
        self.requested = None;
    }
    pub fn is_open(&self) -> bool {
        self.requested.is_some()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deletion_requires_distinct_request_and_single_matching_confirmation() {
        let mut gate = ConfirmationGate::default();
        assert!(!gate.take("review-1", false, true));
        assert!(gate.request("review-1", false, true));
        assert!(gate.is_open());
        assert!(gate.take("review-1", false, true));
        assert!(!gate.take("review-1", false, true));
    }
    #[test]
    fn changed_review_identity_or_blocked_review_never_confirms() {
        let mut gate = ConfirmationGate::default();
        assert!(!gate.request("blocked", false, false));
        assert!(gate.request("manual", false, true));
        assert!(!gate.take("cleanup", true, true));
        assert!(!gate.take("manual", false, true));
        assert!(gate.request("manual", false, true));
        assert!(!gate.take("manual", false, false));
    }
    #[test]
    fn cancel_discards_confirmation() {
        let mut gate = ConfirmationGate::default();
        assert!(gate.request("review", true, true));
        gate.dismiss();
        assert!(!gate.take("review", true, true));
    }
}
