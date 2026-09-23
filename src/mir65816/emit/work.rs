//! Qualification-only counts of work actually performed on the calling thread.
use std::{cell::RefCell, collections::BTreeMap};

pub type Counts = BTreeMap<&'static str, u64>;
thread_local! {
    static ACTIVE: RefCell<Option<Counts>> = const { RefCell::new(None) };
}

pub(super) fn add(kind: &'static str, count: usize) {
    ACTIVE.with_borrow_mut(|active| {
        if let Some(counts) = active {
            *counts.entry(kind).or_default() += count as u64;
        }
    });
}

/// One synchronous compilation/query scope. No process-wide counter state;
/// nested scopes are rejected and unwinding cannot contaminate the next run.
pub fn measure<T>(run: impl FnOnce() -> T) -> (T, Counts) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            ACTIVE.with_borrow_mut(|active| *active = None);
        }
    }
    ACTIVE.with_borrow_mut(|active| {
        assert!(active.is_none(), "nested native work measurement");
        *active = Some(Counts::new());
    });
    let reset = Reset;
    let value = run();
    let counts = ACTIVE.with_borrow_mut(|active| active.take().unwrap());
    drop(reset);
    (value, counts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_are_isolated_across_threads_and_unwinding() {
        let (_, counts) = measure(|| {
            add("outer", 2);
            let other = std::thread::spawn(|| measure(|| add("other", 3)))
                .join()
                .unwrap();
            assert_eq!(other.1, [("other", 3)].into());
            assert!(std::panic::catch_unwind(|| measure(|| ())).is_err());
            add("outer", 1);
        });
        assert_eq!(counts, [("outer", 3)].into());
        assert!(std::panic::catch_unwind(|| measure(|| panic!("control"))).is_err());
        assert!(measure(|| ()).1.is_empty());
        add("outside", 1);
        assert!(measure(|| ()).1.is_empty());
    }
}
