use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(target_os = "macos")]
fn fixture_record() -> Vec<u8> {
    let mut bytes = vec![0; 40];
    for (at, value) in [(0, 40_u32), (4, 0xa000_0001), (28, 8), (32, 2)] {
        bytes[at..at + 4].copy_from_slice(&value.to_ne_bytes());
    }
    bytes[36] = b'x';
    bytes
}

#[cfg(target_os = "linux")]
fn fixture_record() -> Vec<u8> {
    let mut bytes = vec![0; 24];
    bytes[0] = 1;
    bytes[16..18].copy_from_slice(&24_u16.to_ne_bytes());
    bytes[18] = 8;
    bytes[19] = b'x';
    bytes
}

#[test]
fn decoder_bounds_and_mutations() {
    let good = fixture_record();
    let record = decode(&good).unwrap();
    assert_eq!(&good[record.name], b"x");
    for end in 0..good.len() {
        assert!(decode(&good[..end]).is_err());
    }
    // Vary each input byte independently; even valid mutations must not produce
    // a reference outside the record, include a separator/NUL, or stall progress.
    for at in 0..good.len() {
        for value in 0..=u8::MAX {
            let mut bytes = good.clone();
            bytes[at] = value;
            if let Ok(record) = decode(&bytes) {
                assert!(record.length > 0 && record.length <= bytes.len());
                assert!(record.name.start < record.name.end && record.name.end < record.length);
                assert!(bytes[record.name].iter().all(|b| *b != 0 && *b != b'/'));
            }
        }
    }
}

#[cfg(target_os = "macos")]
#[test]
fn decoder_flags_offsets_and_native_error() {
    for (at, value) in [
        (0, 0),
        (0, u32::MAX),
        (0, 41),
        (4, 0),
        (4, 0xa000_0003),
        (8, 1),
        (28, u32::MAX),
        (28, 0),
        (28, 40),
        (32, 0),
        (32, 1),
        (32, u32::MAX),
    ] {
        let mut bytes = fixture_record();
        bytes[at..at + 4].copy_from_slice(&value.to_ne_bytes());
        assert!(decode(&bytes).is_err());
    }
    let mut bytes = fixture_record();
    bytes[24..28].copy_from_slice(&13_u32.to_ne_bytes());
    assert_eq!(decode(&bytes).err().unwrap().raw_os_error(), Some(13));
    let mut bytes = fixture_record();
    bytes[37] = b'x';
    assert!(decode(&bytes).is_err());
}

#[test]
fn decoder_reviewed_abi() {
    assert_eq!(std::mem::align_of::<DirectoryBuffer>(), 8);
    assert_eq!(std::mem::size_of::<DirectoryBuffer>(), BUFFER_BYTES);

    #[cfg(target_os = "macos")]
    {
        assert_eq!(std::mem::size_of::<libc::stat>(), 144);
        assert_eq!(std::mem::align_of::<libc::stat>(), 8);
        assert_eq!(std::mem::offset_of!(libc::stat, st_dev), 0);
        assert_eq!(std::mem::offset_of!(libc::stat, st_mode), 4);
        assert_eq!(std::mem::offset_of!(libc::stat, st_nlink), 6);
        assert_eq!(std::mem::offset_of!(libc::stat, st_ino), 8);
        assert_eq!(std::mem::offset_of!(libc::stat, st_size), 96);
        assert_eq!(std::mem::offset_of!(libc::stat, st_mtime), 48);
        assert_eq!(std::mem::offset_of!(libc::stat, st_mtime_nsec), 56);
        assert_eq!(std::mem::offset_of!(libc::stat, st_ctime), 64);
        assert_eq!(std::mem::offset_of!(libc::stat, st_ctime_nsec), 72);
        assert_eq!(libc::PATH_MAX, 1024);
        assert_eq!(std::mem::size_of::<libc::attrlist>(), 24);
        assert_eq!(std::mem::align_of::<libc::attrlist>(), 4);
        assert_eq!(std::mem::offset_of!(libc::attrlist, commonattr), 4);
        assert_eq!(std::mem::offset_of!(libc::attrlist, forkattr), 20);
        assert_eq!(
            libc::ATTR_CMN_RETURNED_ATTRS | ATTR_CMN_ERROR | libc::ATTR_CMN_NAME,
            0xa000_0001
        );
        assert_eq!(libc::ATTR_BIT_MAP_COUNT, 5);
        assert_eq!(libc::FSOPT_PACK_INVAL_ATTRS, 8);
        assert_eq!(libc::O_RDONLY, 0);
        assert_eq!(libc::O_NONBLOCK, 4);
        assert_eq!(libc::O_DIRECTORY, 0x100000);
        assert_eq!(libc::O_CLOEXEC, 0x1000000);
    }

    #[cfg(target_os = "linux")]
    {
        assert_eq!(std::mem::offset_of!(libc::dirent64, d_ino), 0);
        assert_eq!(std::mem::offset_of!(libc::dirent64, d_reclen), 16);
        assert_eq!(std::mem::offset_of!(libc::dirent64, d_name), 19);
    }
}

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Temp(std::path::PathBuf);

impl Temp {
    fn new() -> Self {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("pipesql-directory-{}-{id}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[cfg(target_os = "macos")]
#[test]
fn native_canonical_names_match_reference_on_joined_workers() {
    let root = Temp::new();
    let base = std::fs::canonicalize(&root.0).unwrap();
    let directory = base.join("CaseDir");
    std::fs::create_dir(&directory).unwrap();
    let file = directory.join("café");
    std::fs::write(&file, b"owned fixture").unwrap();
    let relative = base.join("relative");
    let absolute = base.join("absolute");
    std::os::unix::fs::symlink("CaseDir", &relative).unwrap();
    std::os::unix::fs::symlink(&directory, &absolute).unwrap();
    let hardlink = base.join("hardlink");
    std::fs::hard_link(&file, &hardlink).unwrap();
    std::os::unix::fs::symlink("missing", base.join("dangling")).unwrap();
    std::os::unix::fs::symlink("cycle", base.join("cycle")).unwrap();
    let mut repeated_slash = base.as_os_str().to_os_string();
    repeated_slash.push("//CaseDir/");
    let mut paths = vec![
        std::path::PathBuf::from("/"),
        base.clone(),
        directory.clone(),
        file,
        relative,
        absolute,
        hardlink.clone(),
        directory.join(".."),
        directory.join("./"),
        repeated_slash.into(),
        base.join("casedir"),
        base.join("CaseDir/cafe\u{0301}"),
        base.join("dangling"),
        base.join("cycle"),
        std::path::Path::new("/System/Volumes/Data").join(base.strip_prefix("/").unwrap()),
    ];
    // Keep all 33 reviewed source-corpus geometries in the ordinary production
    // gate, rather than certifying only an easy subset of a disposable walker.
    for absolute in [false, true] {
        let mut previous = directory.clone();
        for depth in 1..=34 {
            let link = base.join(format!("link-{absolute}-{depth}"));
            if absolute {
                std::os::unix::fs::symlink(&previous, &link).unwrap();
            } else {
                std::os::unix::fs::symlink(previous.file_name().unwrap(), &link).unwrap();
            }
            previous = link;
            if depth >= 31 {
                let expected = std::fs::canonicalize(&previous);
                if depth <= 33 {
                    assert!(expected.is_ok());
                } else {
                    assert_eq!(expected.unwrap_err().raw_os_error(), Some(libc::ELOOP));
                }
                paths.push(previous.clone());
            }
        }
    }
    for length in [1023, 1024, 1025, 4096] {
        paths.push(std::path::PathBuf::from("/".repeat(length)));
    }
    for suffix in ["/", "/.", "/..", "/child"] {
        let mut path = hardlink.as_os_str().to_os_string();
        path.push(suffix);
        paths.push(path.into());
    }
    paths.extend([
        std::path::PathBuf::from("/dev"),
        std::path::PathBuf::from("/dev/null"),
    ]);
    assert_eq!(paths.len(), 33);
    // The reference and physical checks use std/OS independently of this decoder.
    let expected: Vec<_> = paths.iter().map(std::fs::canonicalize).collect();
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let paths = &paths;
            let expected = &expected;
            scope.spawn(move || {
                for _ in 0..8 {
                    for (path, expected) in paths.iter().zip(expected) {
                        match (canonicalize(path), expected) {
                            (Ok(observed), Ok(expected)) => {
                                assert_eq!(&observed, expected);
                                assert_eq!(
                                    FileIdentity::from_metadata(
                                        &std::fs::metadata(observed).unwrap()
                                    ),
                                    FileIdentity::from_metadata(
                                        &std::fs::metadata(expected).unwrap()
                                    ),
                                );
                            }
                            (Err(CanonicalizeError::Io(observed)), Err(expected)) => {
                                assert_eq!(observed.raw_os_error(), expected.raw_os_error());
                            }
                            pair => panic!("canonical name mismatch for {path:?}: {pair:?}"),
                        }
                    }
                }
            });
        }
    });
}

#[cfg(target_os = "macos")]
#[test]
fn native_deep_absolute_links_preserve_the_33_link_boundary() {
    // Cleanup is iterative even if an assertion fails; input depth never becomes
    // recursive test teardown. The remaining Temp tree is shallow.
    struct Deep {
        root: Temp,
        directories: Vec<std::path::PathBuf>,
        files: Vec<std::path::PathBuf>,
    }

    impl Drop for Deep {
        fn drop(&mut self) {
            for path in self.files.iter().rev() {
                // Preserve an earlier test failure while cleaning owned fixtures.
                let _ = std::fs::remove_file(path);
            }
            for path in self.directories.iter().rev() {
                let _ = std::fs::remove_dir(path);
            }
        }
    }
    let mut fixture = Deep {
        root: Temp::new(),
        directories: Vec::with_capacity(466),
        files: Vec::with_capacity(35),
    };
    // Canonicalize setup with the independent reference so /var's own symlink
    // does not silently turn the intended 33-link input into a 34-link input.
    let mut parent = std::fs::canonicalize(&fixture.root.0).unwrap();
    let depth = (1023_usize
        .checked_sub(parent.as_os_str().as_bytes().len() + 4)
        .unwrap()
        / 2)
    .min(466);
    assert!(
        depth >= 128,
        "temporary root must leave room for the deep-prefix fixture"
    );
    for _ in 0..depth {
        parent.push("a");
        std::fs::create_dir(&parent).unwrap();
        fixture.directories.push(parent.clone());
    }
    let leaf = parent.join("end");
    std::fs::write(&leaf, b"end").unwrap();
    fixture.files.push(leaf.clone());
    let mut target = leaf.clone();
    for index in (0..34).rev() {
        let link = parent.join(format!("l{index}"));
        std::os::unix::fs::symlink(&target, &link).unwrap();
        fixture.files.push(link.clone());
        target = link;
    }
    let admitted = parent.join("l1");
    assert_eq!(
        canonicalize(&admitted).unwrap(),
        std::fs::canonicalize(&admitted).unwrap()
    );
    assert_eq!(
        canonicalize(&admitted).unwrap(),
        std::fs::canonicalize(&leaf).unwrap()
    );
    let refused = parent.join("l0");
    assert!(
        matches!(canonicalize(&refused), Err(CanonicalizeError::Io(error)) if error.raw_os_error() == Some(libc::ELOOP))
    );
    assert_eq!(
        std::fs::canonicalize(&refused).unwrap_err().raw_os_error(),
        Some(libc::ELOOP)
    );
}

#[test]
fn native_directory_names_refusal_and_independent_cursors() {
    let root = Temp::new();
    for count in [0, 1, 31, 32, 65] {
        let path = root.0.join(count.to_string());
        std::fs::create_dir(&path).unwrap();
        for index in 0..count {
            let name = format!("{index:03}-{}", "x".repeat(251));
            let path = path.join(name);
            if index == 0 {
                std::fs::create_dir(path).unwrap();
            } else if index == 1 {
                std::os::unix::fs::symlink("missing", path).unwrap();
            } else {
                std::fs::write(path, []).unwrap();
            }
        }
        let mut buffer = DirectoryBuffer::default();
        let mut directory = Directory::open(&path, &mut buffer).unwrap();
        let mut names = Vec::new();
        let mut refused = false;
        for _ in 0..=MAX_RAW_ENTRIES {
            match directory.next_name() {
                Ok(Some(name)) => names.push(name.to_owned()),
                Ok(None) => break,
                Err(error) => {
                    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
                    refused = true;
                    break;
                }
            }
        }
        assert!(directory.next_name().unwrap().is_none());
        if count == 65 {
            assert!(refused);
        } else {
            assert!(!refused);
            let mut expected: Vec<_> = std::fs::read_dir(&path)
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            names.sort();
            expected.sort();
            assert_eq!(names, expected);
            // A second reader must not inherit the first reader's EOF offset.
            drop(directory);
            let mut reader = Directory::open(&path, &mut buffer).unwrap();
            assert_eq!(reader.next_name().unwrap().is_some(), count != 0);
        }
    }
}

#[test]
fn native_directory_cursors_are_independent_on_real_threads() {
    let root = Temp::new();
    for index in 0..31 {
        std::fs::write(root.0.join(index.to_string()), []).unwrap();
    }
    std::thread::scope(|scope| {
        let readers: [_; 4] = std::array::from_fn(|_| {
            scope.spawn(|| {
                let mut buffer = DirectoryBuffer::default();
                let mut reader = Directory::open(&root.0, &mut buffer).unwrap();
                let mut names = Vec::new();
                while let Some(name) = reader.next_name().unwrap() {
                    names.push(name.to_owned());
                }
                names.sort();
                assert_eq!(names.len(), 31);
                names
            })
        });
        let mut expected = None;
        for reader in readers {
            let names = reader.join().unwrap();
            if let Some(expected) = &expected {
                assert_eq!(&names, expected);
            } else {
                expected = Some(names);
            }
        }
    });
}

#[test]
fn native_path_metadata_matches_independent_std_and_opened_files() {
    use std::io::{Read, Write};
    use std::os::unix::fs::MetadataExt;
    let root = Temp::new();
    let directory = root.0.join("x".repeat(200)).join("y".repeat(200));
    std::fs::create_dir_all(&directory).unwrap();
    let file = directory.join("file");
    let alias = directory.join("alias");
    let link = directory.join("symlink");
    let missing = directory.join("missing");
    std::fs::write(&file, b"retained").unwrap();
    std::fs::hard_link(&file, &alias).unwrap();
    std::os::unix::fs::symlink("file", &link).unwrap();
    std::os::unix::fs::symlink("absent", &missing).unwrap();
    for path in [&directory, &file, &alias, &link, &missing] {
        let expected = std::fs::symlink_metadata(path).unwrap();
        let actual = symlink_metadata(path).unwrap();
        assert_eq!(actual.identity(), FileIdentity::from_metadata(&expected));
        assert_eq!(actual.len(), expected.len());
        assert_eq!(actual.nlink(), expected.nlink());
        let normalized = Metadata::from_std(&expected);
        assert_eq!(
            (
                actual.mtime(),
                actual.mtime_nsec(),
                actual.ctime(),
                actual.ctime_nsec()
            ),
            (
                normalized.mtime(),
                normalized.mtime_nsec(),
                normalized.ctime(),
                normalized.ctime_nsec()
            )
        );
        assert_eq!(actual.file_type().is_file(), expected.file_type().is_file());
        assert_eq!(actual.file_type().is_dir(), expected.file_type().is_dir());
        assert_eq!(
            actual.file_type().is_symlink(),
            expected.file_type().is_symlink()
        );
    }
    let mut opened = open_read(&file).unwrap();
    let observed = file_metadata(&opened).unwrap();
    let expected = opened.metadata().unwrap();
    assert_eq!(
        observed.identity(),
        symlink_metadata(&alias).unwrap().identity()
    );
    assert_eq!(observed.len(), expected.len());
    assert_eq!(observed.nlink(), expected.nlink());
    assert_eq!(observed.mtime(), expected.mtime());
    assert_eq!(observed.mtime_nsec(), expected.mtime_nsec());
    assert_eq!(observed.ctime(), expected.ctime());
    assert_eq!(observed.ctime_nsec(), expected.ctime_nsec());
    assert!(observed.file_type().is_file());
    let mut bytes = [0; 8];
    opened.read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"retained");
    assert!(opened.write_all(b"bad").is_err());
    let mut opened = open_read_write(&file).unwrap();
    assert_eq!(opened.metadata().unwrap().len(), 8);
    opened.write_all(b"changed!").unwrap();
    assert_eq!(std::fs::read(&alias).unwrap(), b"changed!");
    // An open descriptor follows its original object after its name is replaced.
    std::fs::remove_file(&file).unwrap();
    std::fs::write(&file, b"replacement").unwrap();
    let retained = file_metadata(&opened).unwrap();
    assert_eq!(retained.identity(), observed.identity());
    assert_ne!(
        retained.identity(),
        symlink_metadata(&file).unwrap().identity()
    );
    assert_eq!(retained.nlink(), 1);
    assert_eq!(retained.len(), 8);
    assert_eq!(
        open_read_write(directory.join("not-created"))
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
    assert!(!directory.join("not-created").exists());
    assert_eq!(
        open_read(&missing).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    for name in [b"bad\0path".to_vec(), vec![b'x'; MAX_PATH_BYTES + 1]] {
        let path = Path::new(OsStr::from_bytes(&name));
        assert_eq!(
            symlink_metadata(path).err().unwrap().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            open_read(path).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            open_read_write(path).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
}

#[test]
fn native_path_mutations_and_canonicalization_match_std() {
    use std::io::Write;
    use std::os::unix::fs::MetadataExt;
    let root = Temp::new();
    let long = root.0.join("a".repeat(200)).join("b".repeat(200));
    std::fs::create_dir_all(&long).unwrap();
    let directory = long.join("directory");
    create_dir(&directory).unwrap();
    let input = directory.join("input");
    let output = directory.join("output");
    let alias = directory.join("alias");
    let mut file = create_new_read_write(&input).unwrap();
    file.write_all(b"source bytes").unwrap();
    drop(file);
    assert_eq!(
        create_new_read_write(&input).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(std::fs::read(&input).unwrap(), b"source bytes");
    let std_created = directory.join("std-created");
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&std_created)
        .unwrap();
    assert_eq!(
        std::fs::metadata(&input).unwrap().mode(),
        std::fs::metadata(&std_created).unwrap().mode()
    );
    hard_link(&input, &alias).unwrap();
    assert_eq!(symlink_metadata(&input).unwrap().nlink(), 2);
    assert_eq!(
        hard_link(&input, &alias).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    std::fs::write(&output, b"destination bytes").unwrap();
    rename(&input, &output).unwrap();
    assert!(!input.exists());
    assert_eq!(std::fs::read(&output).unwrap(), b"source bytes");
    assert_eq!(
        symlink_metadata(&alias).unwrap().identity(),
        symlink_metadata(&output).unwrap().identity()
    );
    let symlink = directory.join("symlink");
    std::os::unix::fs::symlink("output", &symlink).unwrap();
    assert_eq!(
        metadata(&symlink).unwrap().identity(),
        symlink_metadata(&output).unwrap().identity()
    );
    for path in [
        &directory,
        &output,
        &symlink,
        &directory.join("../directory/output"),
    ] {
        assert_eq!(
            canonicalize(path).unwrap(),
            std::fs::canonicalize(path).unwrap()
        );
    }
    assert!(matches!(
        canonicalize(Path::new("relative")),
        Err(CanonicalizeError::Io(error)) if error.kind() == io::ErrorKind::InvalidInput
    ));
    for invalid in [b"bad\0path".to_vec(), vec![b'x'; MAX_PATH_BYTES + 1]] {
        let path = Path::new(OsStr::from_bytes(&invalid));
        assert!(rename(&output, path).is_err());
        assert!(hard_link(&output, path).is_err());
        assert!(create_dir(path).is_err());
        assert!(create_new_read_write(path).is_err());
        assert!(remove_file(path).is_err());
        assert!(remove_dir(path).is_err());
        assert_eq!(std::fs::read(&output).unwrap(), b"source bytes");
    }
    assert!(remove_dir(&directory).is_err());
    for path in [&output, &alias, &symlink, &std_created] {
        remove_file(path).unwrap();
    }
    remove_dir(&directory).unwrap();
    assert!(!directory.exists());
}

#[test]
fn native_directory_open_errors_and_long_path() {
    let root = Temp::new();
    let mut buffer = DirectoryBuffer::default();
    let file = root.0.join("file");
    std::fs::write(&file, []).unwrap();
    assert_eq!(
        Directory::open(&file, &mut buffer).err().unwrap().kind(),
        io::ErrorKind::NotADirectory
    );
    let nul = Path::new(OsStr::from_bytes(b"bad\0path"));
    assert_eq!(
        Directory::open(nul, &mut buffer).err().unwrap().kind(),
        io::ErrorKind::InvalidInput
    );
    let excessive = "x".repeat(MAX_PATH_BYTES + 1);
    assert_eq!(
        Directory::open(Path::new(&excessive), &mut buffer)
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    let long = root.0.join("x".repeat(200)).join("y".repeat(200));
    std::fs::create_dir_all(&long).unwrap();
    std::fs::write(long.join("entry"), []).unwrap();
    let mut directory = Directory::open(&long, &mut buffer).unwrap();
    assert_eq!(directory.next_name().unwrap(), Some(OsStr::new("entry")));
    assert!(directory.next_name().unwrap().is_none());
}

#[test]
fn directory_total_bound_is_independent_of_per_call_work() {
    let root = Temp::new();
    for index in 0..80 {
        std::fs::write(root.0.join(format!("{index:03}-{}", "x".repeat(251))), []).unwrap();
    }
    let mut buffer = DirectoryBuffer::default();
    let mut reader = Directory::open_bounded(&root.0, &mut buffer, 82).unwrap();
    let mut count = 0;
    loop {
        let before = reader.raw_entries;
        let next = reader.next_name().unwrap().is_some();
        assert!(reader.raw_entries - before <= MAX_RAW_ENTRIES + 1);
        if !next {
            break;
        }
        count += 1;
    }
    assert_eq!(count, 80);
    let exact = reader.raw_entries;
    drop(reader);
    for (limit, success) in [(exact, true), (exact - 1, false), (64, false)] {
        let mut reader = Directory::open_bounded(&root.0, &mut buffer, limit).unwrap();
        let mut observed = 0;
        let finished = loop {
            match reader.next_name() {
                Ok(Some(_)) => observed += 1,
                Ok(None) => break true,
                Err(error) => {
                    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
                    break false;
                }
            }
        };
        assert_eq!(finished, success);
        if success {
            assert_eq!(observed, 80);
        }
        assert!(reader.next_name().unwrap().is_none());
    }
    assert!(
        matches!(Directory::open_bounded(&root.0, &mut buffer, 0), Err(e) if e.kind() == io::ErrorKind::InvalidInput)
    );
}
