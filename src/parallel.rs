//! Ordered scoped parallelism built on the Rust standard library.

use std::cell::Cell;
use std::sync::OnceLock;

thread_local! {
    static THREAD_OVERRIDE: Cell<usize> = const { Cell::new(0) };
    static INSIDE_WORKER: Cell<bool> = const { Cell::new(false) };
}

static DEFAULT_THREAD_COUNT: OnceLock<usize> = OnceLock::new();

pub(crate) fn current_num_threads() -> usize {
    if INSIDE_WORKER.with(Cell::get) {
        return 1;
    }
    let overridden = THREAD_OVERRIDE.with(Cell::get);
    if overridden != 0 {
        overridden
    } else {
        *DEFAULT_THREAD_COUNT.get_or_init(configured_thread_count)
    }
}

pub(crate) fn with_thread_count<R>(count: usize, operation: impl FnOnce() -> R) -> R {
    THREAD_OVERRIDE.with(|slot| {
        let previous = slot.replace(count.max(1));
        let _reset = CellReset { slot, previous };
        operation()
    })
}

pub(crate) fn map_ordered<T, R, F>(items: &[T], operation: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    map_indexed_ordered(items, |_, item| operation(item))
}

pub(crate) fn map_indexed_ordered<T, R, F>(items: &[T], operation: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(usize, &T) -> R + Sync,
{
    let worker_count = current_num_threads().min(items.len());
    if worker_count <= 1 {
        return items
            .iter()
            .enumerate()
            .map(|(index, item)| operation(index, item))
            .collect();
    }

    let chunk_len = items.len().div_ceil(worker_count);
    let mut completed = std::thread::scope(|scope| {
        let (sender, receiver) = std::sync::mpsc::channel();
        for (chunk_index, chunk) in items.chunks(chunk_len).enumerate() {
            let sender = sender.clone();
            let operation = &operation;
            let start = chunk_index * chunk_len;
            scope.spawn(move || {
                let _worker = WorkerGuard::enter();
                let values = chunk
                    .iter()
                    .enumerate()
                    .map(|(offset, item)| operation(start + offset, item))
                    .collect::<Vec<_>>();
                if sender.send((start, values)).is_err() {
                    panic!("parallel result receiver disconnected before workers completed");
                }
            });
        }
        drop(sender);
        receiver.into_iter().collect::<Vec<_>>()
    });
    completed.sort_unstable_by_key(|(start, _)| *start);
    completed
        .into_iter()
        .flat_map(|(_, values)| values)
        .collect()
}

pub(crate) fn for_each_indexed<T, F>(items: &[T], operation: F)
where
    T: Sync,
    F: Fn(usize, &T) + Sync,
{
    let _: Vec<()> = map_indexed_ordered(items, |index, item| operation(index, item));
}

pub(crate) fn fold_reduce<T, A, Init, Fold, Reduce>(
    items: &[T],
    init: Init,
    fold: Fold,
    reduce: Reduce,
) -> A
where
    T: Sync,
    A: Send,
    Init: Fn() -> A + Sync,
    Fold: Fn(A, &T) -> A + Sync,
    Reduce: Fn(A, A) -> A + Sync,
{
    if items.is_empty() {
        return init();
    }
    let ranges = chunk_ranges(items.len(), current_num_threads());
    let partials = map_ordered(&ranges, |&(start, end)| {
        items[start..end]
            .iter()
            .fold(init(), |accumulator, item| fold(accumulator, item))
    });
    partials.into_iter().fold(init(), reduce)
}

fn chunk_ranges(item_count: usize, worker_count: usize) -> Vec<(usize, usize)> {
    if item_count == 0 {
        return Vec::new();
    }
    let chunk_len = item_count.div_ceil(worker_count.max(1).min(item_count));
    (0..item_count)
        .step_by(chunk_len)
        .map(|start| (start, (start + chunk_len).min(item_count)))
        .collect()
}

fn configured_thread_count() -> usize {
    ["FULLBLEED_THREADS", "RAYON_NUM_THREADS"]
        .iter()
        .find_map(|name| {
            std::env::var(name)
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|value| *value != 0)
        })
        .or_else(|| std::thread::available_parallelism().ok().map(usize::from))
        .unwrap_or(1)
}

struct CellReset<'a> {
    slot: &'a Cell<usize>,
    previous: usize,
}

impl Drop for CellReset<'_> {
    fn drop(&mut self) {
        self.slot.set(self.previous);
    }
}

struct WorkerGuard {
    previous: bool,
}

impl WorkerGuard {
    fn enter() -> Self {
        Self {
            previous: INSIDE_WORKER.with(|slot| slot.replace(true)),
        }
    }
}

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        INSIDE_WORKER.with(|slot| slot.set(self.previous));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Mutex;

    #[test]
    fn mapping_preserves_input_order() {
        let values: Vec<usize> = (0..257).collect();
        let mapped = with_thread_count(4, || {
            map_indexed_ordered(&values, |index, value| (index, value * value))
        });
        assert_eq!(mapped.len(), values.len());
        for (index, (reported_index, square)) in mapped.into_iter().enumerate() {
            assert_eq!(reported_index, index);
            assert_eq!(square, index * index);
        }
    }

    #[test]
    fn explicit_parallelism_controls_worker_count() {
        let values: Vec<usize> = (0..128).collect();
        let thread_ids = Mutex::new(HashSet::new());
        with_thread_count(4, || {
            map_ordered(&values, |_| {
                thread_ids
                    .lock()
                    .expect("thread id set")
                    .insert(std::thread::current().id());
            });
        });
        assert_eq!(thread_ids.into_inner().expect("thread id set").len(), 4);
    }

    #[test]
    fn fold_reduce_combines_every_item() {
        let values: Vec<u64> = (1..=10_000).collect();
        let total = with_thread_count(4, || {
            fold_reduce(&values, || 0u64, |sum, value| sum + value, |a, b| a + b)
        });
        assert_eq!(total, 50_005_000);
    }
}
