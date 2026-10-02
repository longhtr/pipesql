//! Check native mutex behavior and thread stack bounds against installed C headers.
//!
//! The platform driver compares Rust layouts with the separately compiled SDK
//! witness, exercises a normal mutex, and observes native thread stack extents.
//! The oversized thread is a control: the same observation must reject it against
//! the ceiling. Stack extent is not the deepest stack use of database code.
//! Failures terminate this isolated process rather than dropping storage that a
//! native thread or initialized lock may still use. Observer-mode checks have a
//! separate dispatch path in platform_observer.

use std::{
    io::Write,
    mem::{MaybeUninit, align_of, size_of},
};

mod platform_observer;

unsafe extern "C" {
    fn pipesql_header_mutex_size() -> usize;
    fn pipesql_header_mutex_alignment() -> usize;
    fn pipesql_header_stack_minimum() -> usize;
    fn pipesql_header_signatures();
}

fn require(condition: bool, operation: &str) {
    if !condition {
        let _ = writeln!(std::io::stderr(), "platform check failed: {operation}");
        // This isolated driver may still own a native thread or initialized lock.
        // Terminating the process keeps failure from dropping storage still in use.
        std::process::exit(1);
    }
}

fn success(result: libc::c_int, operation: &str) {
    if result != 0 {
        let _ = writeln!(
            std::io::stderr(),
            "{operation}: {}",
            std::io::Error::from_raw_os_error(result)
        );
        std::process::exit(1);
    }
}

fn mutex() {
    let mut attributes = MaybeUninit::uninit();
    let mut mutex = MaybeUninit::uninit();
    // SAFETY: storage stays at these addresses from initialization through
    // destruction. Calls are synchronous; no other thread receives this mutex.
    unsafe {
        require(
            pipesql_header_mutex_size() == size_of::<libc::pthread_mutex_t>(),
            "mutex size",
        );
        require(
            pipesql_header_mutex_alignment() == align_of::<libc::pthread_mutex_t>(),
            "mutex alignment",
        );
        pipesql_header_signatures();
        success(
            libc::pthread_mutexattr_init(attributes.as_mut_ptr()),
            "mutex attributes",
        );
        success(
            libc::pthread_mutexattr_settype(attributes.as_mut_ptr(), libc::PTHREAD_MUTEX_NORMAL),
            "normal mutex type",
        );
        success(
            libc::pthread_mutex_init(mutex.as_mut_ptr(), attributes.as_ptr()),
            "mutex initialization",
        );
        success(
            libc::pthread_mutexattr_destroy(attributes.as_mut_ptr()),
            "destroy mutex attributes",
        );
        success(libc::pthread_mutex_lock(mutex.as_mut_ptr()), "lock");
        require(
            libc::pthread_mutex_trylock(mutex.as_mut_ptr()) == libc::EBUSY,
            "held normal mutex refuses try-lock",
        );
        success(libc::pthread_mutex_unlock(mutex.as_mut_ptr()), "unlock");
        success(
            libc::pthread_mutex_destroy(mutex.as_mut_ptr()),
            "destroy mutex",
        );
    }
    println!(
        "mutex: {} bytes, alignment {}",
        size_of::<libc::pthread_mutex_t>(),
        align_of::<libc::pthread_mutex_t>()
    );
}

#[derive(Default)]
struct Observation {
    bytes: usize,
    guard: usize,
    contains_local: bool,
    error: libc::c_int,
}

extern "C" fn observe(argument: *mut libc::c_void) -> *mut libc::c_void {
    // SAFETY: the creator retains exclusive observation storage until join. This
    // callback never panics, and no pointer to its own stack escapes the callback.
    unsafe {
        let result = &mut *argument.cast::<Observation>();
        let local = &argument as *const _ as usize;
        #[cfg(target_os = "macos")]
        {
            result.bytes = libc::pthread_get_stacksize_np(libc::pthread_self());
            let upper = libc::pthread_get_stackaddr_np(libc::pthread_self()) as usize;
            if let Some(base) = upper.checked_sub(result.bytes) {
                result.contains_local = local >= base && local < upper;
            }
        }
        #[cfg(target_os = "linux")]
        {
            let mut attributes = MaybeUninit::uninit();
            result.error = libc::pthread_getattr_np(libc::pthread_self(), attributes.as_mut_ptr());
            if result.error == 0 {
                let mut base = std::ptr::null_mut();
                result.error =
                    libc::pthread_attr_getstack(attributes.as_ptr(), &mut base, &mut result.bytes);
                if result.error == 0 {
                    result.error =
                        libc::pthread_attr_getguardsize(attributes.as_ptr(), &mut result.guard);
                }
                let destroy = libc::pthread_attr_destroy(attributes.as_mut_ptr());
                if result.error == 0 {
                    result.error = destroy;
                }
                result.contains_local = local
                    .checked_sub(base as usize)
                    .is_some_and(|offset| offset < result.bytes);
            }
        }
    }
    std::ptr::null_mut()
}

fn stacks() {
    let mut attributes = MaybeUninit::uninit();
    // SAFETY: initialized attributes stay in place until destruction; the result
    // and thread identifier remain alive until join succeeds. A failed join exits
    // the process before either can go out of scope.
    unsafe {
        success(
            libc::pthread_attr_init(attributes.as_mut_ptr()),
            "thread attributes",
        );
        let minimum = pipesql_header_stack_minimum();
        println!("SDK stack minimum: {minimum}");
        let (small, ceiling) = if cfg!(all(
            target_os = "linux",
            target_arch = "aarch64",
            target_env = "gnu"
        )) {
            require(minimum == 128 * 1024, "GNU arm64 stack minimum");
            for bytes in [48 * 1024, 64 * 1024] {
                require(
                    libc::pthread_attr_setstacksize(attributes.as_mut_ptr(), bytes) == libc::EINVAL,
                    "undersized stack refusal",
                );
            }
            (128 * 1024, 144 * 1024)
        } else {
            (48 * 1024, 64 * 1024)
        };
        for (request, accepted) in [(small, true), (2 * 1024 * 1024, false)] {
            success(
                libc::pthread_attr_setstacksize(attributes.as_mut_ptr(), request),
                "requested stack size",
            );
            let mut result = Observation::default();
            let mut thread = MaybeUninit::uninit();
            success(
                libc::pthread_create(
                    thread.as_mut_ptr(),
                    attributes.as_ptr(),
                    observe,
                    (&mut result as *mut Observation).cast(),
                ),
                "create thread",
            );
            success(
                libc::pthread_join(thread.assume_init(), std::ptr::null_mut()),
                "join thread",
            );
            success(result.error, "observe stack");
            require(
                result.bytes > 0 && result.contains_local,
                "live local lies inside stack bounds",
            );
            require(
                (result.bytes <= ceiling) == accepted,
                "stack ceiling and oversized control",
            );
            println!(
                "stack: requested={request} reported={} guard={} accepted={accepted}",
                result.bytes, result.guard
            );
        }
        success(
            libc::pthread_attr_destroy(attributes.as_mut_ptr()),
            "destroy thread attributes",
        );
    }
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    if let [_, command, first, second, release] = args.as_slice()
        && command == "observer-modes"
    {
        assert!(matches!(release.as_str(), "release" | "overlap"));
        platform_observer::run(first, second, release == "release");
        return;
    }
    require(std::env::args_os().len() == 1, "unexpected arguments");
    mutex();
    stacks();
    println!("platform checks complete");
}
