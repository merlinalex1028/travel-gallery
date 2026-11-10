use std::sync::Mutex;

#[derive(Default)]
enum ExitState {
    #[default]
    Idle,
    Prompting,
    Confirmed,
}

#[derive(Default)]
pub(crate) struct ExitGuard {
    state: Mutex<ExitState>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ExitDecision {
    Allow,
    Prompt,
    WaitForPrompt,
}

impl ExitGuard {
    pub(crate) fn request(&self, unfinished: bool) -> ExitDecision {
        let mut state = self.state.lock().expect("exit state lock poisoned");
        match *state {
            ExitState::Confirmed => ExitDecision::Allow,
            ExitState::Prompting => ExitDecision::WaitForPrompt,
            ExitState::Idle if !unfinished => ExitDecision::Allow,
            ExitState::Idle => {
                *state = ExitState::Prompting;
                ExitDecision::Prompt
            }
        }
    }

    pub(crate) fn resolve(&self, exit: bool) {
        *self.state.lock().expect("exit state lock poisoned") =
            if exit { ExitState::Confirmed } else { ExitState::Idle };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfinished_work_blocks_duplicate_exit_requests_until_answered() {
        let guard = ExitGuard::default();
        assert_eq!(guard.request(true), ExitDecision::Prompt);
        assert_eq!(guard.request(true), ExitDecision::WaitForPrompt);
        guard.resolve(false);
        assert_eq!(guard.request(true), ExitDecision::Prompt);
    }

    #[test]
    fn confirmation_allows_the_followup_programmatic_exit() {
        let guard = ExitGuard::default();
        assert_eq!(guard.request(true), ExitDecision::Prompt);
        guard.resolve(true);
        assert_eq!(guard.request(true), ExitDecision::Allow);
    }

    #[test]
    fn completed_work_needs_no_confirmation() {
        assert_eq!(ExitGuard::default().request(false), ExitDecision::Allow);
    }

    #[test]
    fn finishing_work_does_not_bypass_an_open_confirmation() {
        let guard = ExitGuard::default();
        assert_eq!(guard.request(true), ExitDecision::Prompt);
        assert_eq!(guard.request(false), ExitDecision::WaitForPrompt);
        guard.resolve(false);
        assert_eq!(guard.request(false), ExitDecision::Allow);
    }

    #[test]
    fn concurrent_exit_requests_open_only_one_confirmation() {
        use std::sync::{Arc, Barrier};
        let guard = Arc::new(ExitGuard::default());
        let barrier = Arc::new(Barrier::new(8));
        let threads: Vec<_> = (0..8).map(|_| {
            let guard = guard.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                guard.request(true)
            })
        }).collect();
        let decisions: Vec<_> = threads.into_iter().map(|thread| thread.join().unwrap()).collect();
        assert_eq!(decisions.iter().filter(|d| **d == ExitDecision::Prompt).count(), 1);
        assert_eq!(decisions.iter().filter(|d| **d == ExitDecision::WaitForPrompt).count(), 7);
    }
}
