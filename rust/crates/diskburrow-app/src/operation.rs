//! UI-thread admission and publication. Worker results never become current after cancellation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Purpose {
    Scan { root: String, fast: bool },
    Cleanup,
    Review,
    Save,
    Export,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    Started(u64),
    Joined(u64),
    Busy,
}
#[derive(Default)]
pub struct Coordinator {
    generation: u64,
    active: Option<(u64, Purpose, bool)>,
    pub latest_completed: Option<u64>,
}
impl Coordinator {
    pub fn begin(&mut self, purpose: Purpose) -> Admission {
        if let Some((token, current, cancelled)) = &self.active {
            if !cancelled && same_purpose(current, &purpose) {
                return Admission::Joined(*token);
            }
            return Admission::Busy;
        }
        self.generation = self
            .generation
            .checked_add(1)
            .expect("Operation generation exhausted");
        self.active = Some((self.generation, purpose, false));
        Admission::Started(self.generation)
    }
    pub fn cancel(&mut self) {
        if let Some((_, _, cancelled)) = &mut self.active {
            *cancelled = true;
        }
    }
    pub fn complete(&mut self, token: u64, cancelled: bool) -> bool {
        let Some((active, _, was_cancelled)) = &self.active else {
            return false;
        };
        if *active != token {
            return false;
        }
        let publish = !cancelled && !was_cancelled;
        self.active = None;
        if publish {
            self.latest_completed = Some(token)
        }
        publish
    }
    pub fn is_busy(&self) -> bool {
        self.active.is_some()
    }
}
fn same_purpose(left: &Purpose, right: &Purpose) -> bool {
    let (Purpose::Scan { root: a, fast: af }, Purpose::Scan { root: b, fast: bf }) = (left, right)
    else {
        return false;
    };
    af == bf
        && a.replace('/', "\\")
            .trim_end_matches('\\')
            .eq_ignore_ascii_case(b.replace('/', "\\").trim_end_matches('\\'))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn scan(root: &str, fast: bool) -> Purpose {
        Purpose::Scan {
            root: root.into(),
            fast,
        }
    }
    #[test]
    fn same_root_same_engine_coalesces_and_exclusive_operations_wait() {
        let mut c = Coordinator::default();
        assert_eq!(c.begin(scan("E:\\fixture", false)), Admission::Started(1));
        assert_eq!(c.begin(scan("e:\\FIXTURE\\", false)), Admission::Joined(1));
        assert_eq!(c.begin(scan("E:\\fixture", true)), Admission::Busy);
        assert_eq!(c.begin(Purpose::Cleanup), Admission::Busy);
        assert!(c.complete(1, false));
        assert_eq!(c.begin(Purpose::Cleanup), Admission::Started(2));
    }
    #[test]
    fn cancelled_or_late_result_cannot_change_completed_baseline() {
        let mut c = Coordinator::default();
        c.begin(scan("E:\\fixture", false));
        assert!(c.complete(1, false));
        c.begin(scan("E:\\fixture", false));
        c.cancel();
        assert!(!c.complete(2, false));
        assert_eq!(c.latest_completed, Some(1));
        c.begin(scan("E:\\fixture", false));
        assert!(!c.complete(2, false));
        assert!(c.is_busy());
        assert!(c.complete(3, false));
        assert_eq!(c.latest_completed, Some(3));
    }
    #[test]
    fn worker_reported_cancellation_is_never_published() {
        let mut c = Coordinator::default();
        c.begin(Purpose::Review);
        assert!(!c.complete(1, true));
        assert!(!c.is_busy());
        assert_eq!(c.latest_completed, None);
    }
}
