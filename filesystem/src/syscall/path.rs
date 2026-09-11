//! Call-local Darwin canonical naming. Derived from the reviewed Apple/FreeBSD
//! realpath traversal and suffix-cursor experiment; not a physical-identity lease.
//! Fixed scratch stays private; only a complete checked name escapes on success.
use std::ffi::CStr;
use std::mem::MaybeUninit;

mod name_record;
use name_record as record;

const PATH_BYTES: usize = 1024;
const SYMLINKS: usize = 33;
// Independently asserted by tools/fixtures/filesystem-abi.c.
const NATIVE_SYMLINK_TYPE: u32 = 5;
// Native errno and admission exhaustion are distinct, allocation-free outcomes.
#[derive(Debug, Eq, PartialEq)]
enum Failure {
    Native(i32),
    WorkLimit,
}

impl From<i32> for Failure {
    fn from(error: i32) -> Self {
        Self::Native(error)
    }
}

type Outcome<T> = Result<T, Failure>;

struct NativeWork {
    remaining: u32,
}

impl NativeWork {
    fn new() -> Self {
        Self {
            remaining: crate::MAX_NATIVE_PATH_CALLS,
        }
    }

    fn before_call(&mut self) -> Outcome<()> {
        if self.remaining == 0 {
            return Err(Failure::WorkLimit);
        }
        self.remaining -= 1;
        Ok(())
    }
}

// Optional mount spelling may tolerate a native error, never exhaustion of the
// caller's work admission. This rule applies to every optional helper below.
fn optional_mount_lookup<T>(result: Outcome<T>) -> Outcome<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(Failure::Native(_)) => Ok(None),
        Err(Failure::WorkLimit) => Err(Failure::WorkLimit),
    }
}

struct PathSlot {
    bytes: [u8; PATH_BYTES],
    len: usize,
}

impl PathSlot {
    fn new() -> Self {
        Self {
            bytes: [0; PATH_BYTES],
            len: 0,
        }
    }

    fn view(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    fn cstr(&self) -> &CStr {
        CStr::from_bytes_with_nul(&self.bytes[..=self.len]).expect("slot owns one final NUL")
    }

    fn set(&mut self, bytes: &[u8]) -> Outcome<()> {
        if bytes.len() >= PATH_BYTES {
            return Err(libc::ENAMETOOLONG.into());
        }
        if bytes.contains(&0) {
            return Err(libc::EINVAL.into());
        }
        self.bytes[..bytes.len()].copy_from_slice(bytes);
        self.len = bytes.len();
        self.bytes[self.len] = 0;
        Ok(())
    }

    fn append(&mut self, bytes: &[u8]) -> Outcome<()> {
        let end = self
            .len
            .checked_add(bytes.len())
            .ok_or(libc::ENAMETOOLONG)?;
        if end >= PATH_BYTES {
            return Err(libc::ENAMETOOLONG.into());
        }
        if bytes.contains(&0) {
            return Err(libc::EINVAL.into());
        }
        self.bytes[self.len..end].copy_from_slice(bytes);
        self.len = end;
        self.bytes[end] = 0;
        Ok(())
    }

    fn truncate(&mut self, len: usize) {
        assert!(len <= self.len);
        self.len = len;
        self.bytes[len] = 0;
    }

    fn parent(&mut self) {
        assert!(self.view().starts_with(b"/"));
        if self.len > 1 {
            let end = self.len - usize::from(self.view().ends_with(b"/"));
            let slash = self.bytes[..end].iter().rposition(|b| *b == b'/').unwrap();
            self.truncate(slash + 1);
        }
    }
}

fn errno() -> i32 {
    std::io::Error::last_os_error()
        .raw_os_error()
        .expect("native errno")
}

fn stat(path: &CStr, follow: bool, work: &mut NativeWork) -> Outcome<libc::stat> {
    work.before_call()?;
    let mut raw = MaybeUninit::<libc::stat>::zeroed();
    // SAFETY: live C string and correctly aligned, initialized native storage;
    // synchronous stat/lstat retain neither pointer. Reserved bytes stay zero.
    let result = unsafe {
        if follow {
            libc::stat(path.as_ptr(), raw.as_mut_ptr())
        } else {
            libc::lstat(path.as_ptr(), raw.as_mut_ptr())
        }
    };
    if result < 0 {
        return Err(errno().into());
    }
    // SAFETY: success filled the native scalar fields; all storage was zeroed.
    Ok(unsafe { raw.assume_init() })
}

#[repr(align(8))]
struct NameStorage([u8; record::RECORD_BYTES]);

fn name<'a>(
    path: &CStr,
    storage: &'a mut NameStorage,
    work: &mut NativeWork,
) -> Outcome<record::NameRecord<'a>> {
    work.before_call()?;
    let mut request = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: libc::ATTR_CMN_NAME
            | libc::ATTR_CMN_DEVID
            | libc::ATTR_CMN_OBJTYPE
            | libc::ATTR_CMN_OBJID,
        volattr: 0,
        dirattr: 0,
        fileattr: 0,
        forkattr: 0,
    };
    storage.0.fill(0);
    // SAFETY: SDK-defined request, live C string, aligned writable buffer with
    // exact passed extent. No pointer is retained. Decode validates all offsets.
    let result = unsafe {
        libc::getattrlist(
            path.as_ptr(),
            std::ptr::from_mut(&mut request).cast(),
            storage.0.as_mut_ptr().cast(),
            storage.0.len(),
            libc::FSOPT_NOFOLLOW,
        )
    };
    if result < 0 {
        return Err(errno().into());
    }
    record::decode(&storage.0).ok_or(Failure::Native(libc::EIO))
}

fn read_link(path: &CStr, target: &mut PathSlot, work: &mut NativeWork) -> Outcome<()> {
    work.before_call()?;
    // SAFETY: live C string, exclusive byte buffer of the given length; no
    // retained pointers. readlink returns a byte count, not a terminated string.
    let result =
        unsafe { libc::readlink(path.as_ptr(), target.bytes.as_mut_ptr().cast(), PATH_BYTES) };
    if result < 0 {
        return Err(errno().into());
    }
    let len = usize::try_from(result).map_err(|_| libc::EIO)?;
    if len == 0 {
        return Err(libc::ENOENT.into());
    }
    if len >= PATH_BYTES {
        return Err(libc::ENAMETOOLONG.into());
    }
    if target.bytes[..len].contains(&0) {
        return Err(libc::EIO.into());
    }
    target.len = len;
    target.bytes[len] = 0;
    Ok(())
}

fn mount_name(path: &CStr, mount: &mut PathSlot, work: &mut NativeWork) -> Outcome<()> {
    work.before_call()?;
    let mut raw = MaybeUninit::<libc::statfs>::zeroed();
    // SAFETY: live input and writable target-ABI storage, with no pointer escape.
    if unsafe { libc::statfs(path.as_ptr(), raw.as_mut_ptr()) } < 0 {
        return Err(errno().into());
    }
    // SAFETY: successful syscall and zero-initialized scalar record.
    let raw = unsafe { raw.assume_init() };
    let len = raw
        .f_mntonname
        .iter()
        .position(|b| *b == 0)
        .ok_or(libc::EIO)?;
    if len == 0 || len >= PATH_BYTES || raw.f_mntonname[0].to_ne_bytes()[0] != b'/' {
        return Err(libc::EIO.into());
    }
    for (to, from) in mount.bytes[..len].iter_mut().zip(&raw.f_mntonname[..len]) {
        *to = from.to_ne_bytes()[0];
    }
    mount.len = len;
    mount.bytes[len] = 0;
    Ok(())
}

// Compatibility heuristic only. The legacy object number is NOT a file lease
// identity. Mount-prefix stat errors retain the native naming fallback behavior.
fn matched_mount(
    path: &CStr,
    device: i32,
    object: u64,
    mount: &mut PathSlot,
    work: &mut NativeWork,
) -> Outcome<bool> {
    if optional_mount_lookup(mount_name(path, mount, work))?.is_none() {
        return Ok(false);
    }
    let Some(found) = optional_mount_lookup(stat(mount.cstr(), false, work))? else {
        return Ok(false);
    };
    if found.st_dev != device || found.st_ino != object {
        return Ok(false);
    }
    let mut prefix = PathSlot::new();
    prefix.set(mount.view()).expect("same-capacity copy");
    // Every iteration removes a slash and its suffix, strictly reducing bytes.
    while let Some(slash) = prefix.view().iter().rposition(|b| *b == b'/') {
        if slash == 0 {
            return Ok(true);
        }
        prefix.truncate(slash);
        match optional_mount_lookup(stat(prefix.cstr(), false, work))? {
            Some(value) if value.st_mode & libc::S_IFMT == libc::S_IFDIR => {}
            _ => return Ok(false),
        }
    }
    Ok(false)
}

fn resolve(path: &[u8], output: &mut PathSlot, work: &mut NativeWork) -> Outcome<()> {
    if path.len() > crate::MAX_PATH_BYTES || !path.starts_with(b"/") || path.contains(&0) {
        return Err(libc::EINVAL.into());
    }
    let root_device = stat(c"/", true, work)?.st_dev;
    output.set(b"/")?;
    if path == b"/" {
        return Ok(());
    }
    let mut left = PathSlot::new();
    left.set(&path[1..])?;
    let mut token = PathSlot::new();
    let mut target = PathSlot::new();
    let mut mount = PathSlot::new();
    let mut storage = NameStorage([0; record::RECORD_BYTES]);
    let mut links = 0;
    let mut last_device = root_device;
    // Each ordinary step consumes suffix bytes; each refill consumes one of 33
    // link credits. Input-controlled recursion and suffix shifting are absent.
    let mut cursor = 0;
    while cursor != left.len {
        let suffix = &left.view()[cursor..];
        let slash = suffix.iter().position(|b| *b == b'/');
        let length = slash.unwrap_or(suffix.len());
        token.set(&suffix[..length])?;
        let consumed = length + usize::from(slash.is_some());
        cursor = cursor
            .checked_add(consumed)
            .expect("bounded suffix advancement");
        assert!(cursor <= left.len);
        if !output.view().ends_with(b"/") {
            output.append(b"/")?;
        }
        if token.len == 0 || token.view() == b"." {
            continue;
        }
        if token.view() == b".." {
            output.parent();
            continue;
        }
        let saved = output.len;
        output.append(token.view())?;
        let observed = name(output.cstr(), &mut storage, work);
        let attributes = match observed {
            Ok(value) => Some(value),
            Err(Failure::Native(libc::ENOTSUP | libc::EINVAL)) => None,
            Err(error) => return Err(error),
        };
        let (device, object, is_link) = if let Some(value) = &attributes {
            (
                value.device,
                u64::from(value.legacy_object),
                value.object_type == NATIVE_SYMLINK_TYPE,
            )
        } else {
            let value = stat(output.cstr(), false, work)?;
            (
                value.st_dev,
                value.st_ino,
                value.st_mode & libc::S_IFMT == libc::S_IFLNK,
            )
        };
        if device != last_device {
            last_device = device;
            if matched_mount(output.cstr(), device, object, &mut mount, work)? {
                output.set(mount.view())?;
                continue;
            }
        }
        if is_link {
            if links == SYMLINKS {
                return Err(libc::ELOOP.into());
            }
            links += 1;
            read_link(output.cstr(), &mut target, work)?;
            if target.view().starts_with(b"/") {
                output.set(b"/")?;
                last_device = root_device;
            } else {
                output.parent();
            }
            if slash.is_some() {
                if !target.view().ends_with(b"/") {
                    target.append(b"/")?;
                }
                target.append(&left.view()[cursor..])?;
            }
            left.set(target.view())?;
            cursor = 0;
        } else if let Some(value) = attributes {
            output.truncate(saved);
            output.append(value.name)?;
        }
    }
    if output.len > 1 && output.view().ends_with(b"/") {
        output.truncate(output.len - 1);
    }
    Ok(())
}

pub(super) fn canonicalize(
    path: &[u8],
    destination: &mut [u8],
) -> Result<usize, crate::CanonicalizeError> {
    let mut output = PathSlot::new();
    resolve(path, &mut output, &mut NativeWork::new()).map_err(|error| match error {
        Failure::Native(code) => {
            crate::CanonicalizeError::Io(std::io::Error::from_raw_os_error(code))
        }
        Failure::WorkLimit => crate::CanonicalizeError::WorkLimit,
    })?;
    if output.len == 0 || output.len >= destination.len() {
        return Err(crate::CanonicalizeError::Io(
            std::io::ErrorKind::InvalidData.into(),
        ));
    }
    destination[..=output.len].copy_from_slice(&output.bytes[..=output.len]);
    Ok(output.len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_budget_refuses_the_next_call_and_stays_exhausted() {
        assert_eq!(crate::MAX_NATIVE_PATH_CALLS, 65_536);
        let mut work = NativeWork::new();
        for _ in 0..crate::MAX_NATIVE_PATH_CALLS {
            work.before_call().unwrap();
        }
        assert_eq!(work.before_call(), Err(Failure::WorkLimit));
        assert_eq!(work.before_call(), Err(Failure::WorkLimit));
        assert_eq!(work.remaining, 0);
        assert_eq!(
            optional_mount_lookup::<()>(Err(Failure::WorkLimit)),
            Err(Failure::WorkLimit)
        );
        assert_eq!(
            optional_mount_lookup::<()>(Err(Failure::Native(libc::EIO))),
            Ok(None)
        );
    }

    #[test]
    fn root_resolution_obeys_admission_before_native_effects() {
        let mut output = PathSlot::new();
        output.set(b"/sentinel").unwrap();
        let mut work = NativeWork { remaining: 0 };
        assert_eq!(
            resolve(b"/", &mut output, &mut work),
            Err(Failure::WorkLimit)
        );
        assert_eq!(output.view(), b"/sentinel");
        let mut work = NativeWork { remaining: 1 };
        resolve(b"/", &mut output, &mut work).unwrap();
        assert_eq!(output.view(), b"/");
        assert_eq!(work.remaining, 0);
    }

    #[test]
    fn invalid_input_and_slot_growth_do_not_publish() {
        let mut output = PathSlot::new();
        output.set(b"/sentinel").unwrap();
        for input in [b"".as_slice(), b"relative", b"/bad\0name", &[b'/'; 4097]] {
            let mut work = NativeWork::new();
            assert_eq!(
                resolve(input, &mut output, &mut work),
                Err(Failure::Native(libc::EINVAL))
            );
            assert_eq!(work.remaining, crate::MAX_NATIVE_PATH_CALLS);
            assert_eq!(output.view(), b"/sentinel");
        }
        assert_eq!(
            output.append(&[b'x'; PATH_BYTES]),
            Err(Failure::Native(libc::ENAMETOOLONG))
        );
        assert_eq!(output.view(), b"/sentinel");
        assert_eq!(output.cstr().to_bytes_with_nul(), b"/sentinel\0");
    }
}
