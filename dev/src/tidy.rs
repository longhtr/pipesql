//! Check local links in maintained Markdown; Rust and Cargo check source structure.
//!
//! This scanner supports the inline links and headings used by these documents.
//! It skips fenced examples and external URLs. It does not judge prose or fetch
//! external pages. Reference-style links require extending the scanner first.
//! Inline titles are unsupported; enclose paths with spaces or parentheses in
//! angle brackets. Unsupported forms fail instead of silently losing a target.

use crate::Result;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

fn prose(text: &str) -> Vec<&str> {
    let mut fence = None;
    text.lines()
        .map(|line| {
            let trimmed = line.trim_start_matches(' ');
            let marker = trimmed.as_bytes().first().copied().unwrap_or(0);
            let length = trimmed.bytes().take_while(|byte| *byte == marker).count();
            if line.len() - trimmed.len() <= 3 && matches!(marker, b'`' | b'~') && length >= 3 {
                match fence {
                    None => fence = Some((marker, length)),
                    Some((opening, count))
                        if marker == opening
                            && length >= count
                            && trimmed[length..].trim().is_empty() =>
                    {
                        fence = None
                    }
                    _ => (),
                }
                ""
            } else if fence.is_some() {
                ""
            } else {
                line
            }
        })
        .collect()
}

fn anchors(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let lines = prose(text);
    for (index, line) in lines.iter().enumerate() {
        for attribute in ["id=\"", "id='", "name=\"", "name='"] {
            for part in line.split(attribute).skip(1) {
                if let Some((name, _)) = part.split_once(attribute.chars().last().unwrap()) {
                    found.insert(name.to_owned());
                }
            }
        }
        let line = line.trim();
        let hashes = line.bytes().take_while(|byte| *byte == b'#').count();
        let heading = if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
            Some(line[hashes..].trim().trim_end_matches('#').trim())
        } else if index > 0
            && !line.is_empty()
            && (line.bytes().all(|b| b == b'=') || line.bytes().all(|b| b == b'-'))
        {
            Some(lines[index - 1].trim())
        } else {
            None
        };
        if let Some(heading) = heading {
            let slug: String = heading
                .to_lowercase()
                .chars()
                .filter(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | ' '))
                .map(|c| if c == ' ' { '-' } else { c })
                .collect();
            let mut candidate = slug.clone();
            let mut suffix = 0;
            while found.contains(&candidate) {
                suffix += 1;
                candidate = format!("{slug}-{suffix}");
            }
            found.insert(candidate);
        }
    }
    found
}

fn decode(value: &str) -> Result<String> {
    let mut bytes = Vec::new();
    let mut input = value.bytes();
    while let Some(byte) = input.next() {
        bytes.push(if byte == b'%' {
            let high = input
                .next()
                .and_then(|b| char::from(b).to_digit(16))
                .ok_or("invalid link escape")?;
            let low = input
                .next()
                .and_then(|b| char::from(b).to_digit(16))
                .ok_or("invalid link escape")?;
            (high * 16 + low) as u8
        } else {
            byte
        });
    }
    Ok(String::from_utf8(bytes)?)
}

fn inline_target(text: &str) -> Result<&str> {
    let text = text.trim_start();
    if let Some(text) = text.strip_prefix('<') {
        let (link, rest) = text.split_once('>').ok_or("unclosed inline link")?;
        if !rest.trim_start().starts_with(')') {
            return Err("unsupported inline link suffix".into());
        }
        Ok(link)
    } else {
        let (link, _) = text.split_once(')').ok_or("unclosed inline link")?;
        if link.contains(['(', ' ', '\t']) {
            return Err("use angle brackets for a link containing spaces or parentheses".into());
        }
        Ok(link)
    }
}

fn target(root: &Path, document: &Path, link: &str) -> Result<bool> {
    if link.starts_with("//")
        || link
            .split(['/', '#', '?'])
            .next()
            .unwrap_or("")
            .contains(':')
    {
        return Ok(false);
    }
    let (path, fragment) = link.split_once('#').unwrap_or((link, ""));
    let path = decode(path.split('?').next().unwrap())?;
    let path = if path.is_empty() {
        document.to_owned()
    } else {
        document.parent().unwrap().join(path)
    };
    let path = path
        .canonicalize()
        .map_err(|error| format!("missing target {link}: {error}"))?;
    if !path.starts_with(root) {
        return Err(format!("link escapes repository: {link}").into());
    }
    if !fragment.is_empty()
        && path.extension().is_some_and(|extension| extension == "md")
        && !anchors(&fs::read_to_string(path)?).contains(&decode(fragment)?)
    {
        return Err(format!("missing heading: {link}").into());
    }
    Ok(true)
}

fn documents(directory: &Path, output: &mut Vec<PathBuf>) -> Result<()> {
    if !directory.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            documents(&entry.path(), output)?;
        } else if kind.is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "md")
        {
            output.push(entry.path());
        }
    }
    Ok(())
}

pub fn run() -> Result<()> {
    let root = crate::workspace::root()?.canonicalize()?;
    let mut paths = Vec::new();
    for name in [
        "README.md",
        "DEVELOPMENT.md",
        "THIRD_PARTY.md",
        "vendor/README.md",
    ] {
        if root.join(name).exists() {
            paths.push(root.join(name));
        }
    }
    for directory in ["docs", "src", "filesystem", "test", "dev", "examples"] {
        documents(&root.join(directory), &mut paths)?;
    }
    paths.sort();
    let mut count = 0;
    let mut failures = 0;
    for path in &paths {
        let text = fs::read_to_string(path)?;
        for (number, line) in prose(&text).iter().enumerate() {
            if line.contains("][") || line.trim_start().starts_with('[') && line.contains("]:") {
                eprintln!(
                    "{}:{}: use an inline link so its target is checked",
                    path.display(),
                    number + 1
                );
                failures += 1;
            }
            for part in line.split("](").skip(1) {
                let result = inline_target(part).and_then(|link| target(&root, path, link));
                match result {
                    Ok(local) => count += usize::from(local),
                    Err(error) => {
                        failures += 1;
                        eprintln!("{}:{}: {error}", path.display(), number + 1);
                    }
                }
            }
        }
    }
    println!(
        "repository links: {} documents, {count} local links, {failures} failures",
        paths.len()
    );
    if failures != 0 {
        return Err("broken repository links".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_missing_files_fragments_and_repository_escape() {
        let run = crate::workspace::Run::new(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .to_owned(),
            "link-controls",
        )
        .unwrap();
        let root = run.directory.join("repository");
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let document = root.join("README.md");
        fs::write(&document, "# Present\n").unwrap();
        fs::write(run.directory.join("outside.md"), "outside").unwrap();
        assert!(target(&root, &document, "#present").unwrap());
        assert!(!target(&root, &document, "https://example.com/page#missing").unwrap());
        for link in ["missing.md", "#absent", "../outside.md"] {
            assert!(target(&root, &document, link).is_err(), "{link}");
        }
        std::os::unix::fs::symlink(run.directory.join("outside.md"), root.join("alias.md"))
            .unwrap();
        assert!(target(&root, &document, "alias.md").is_err());
        run.finish().unwrap();
    }

    #[test]
    fn headings_fences_duplicates_and_escapes() {
        let text = "# One `value`\n# One `value`\n```md\n# hidden\n```\nTitle\n=====\n<a id=\"explicit\"></a>";
        assert_eq!(
            anchors(text),
            BTreeSet::from(["one-value", "one-value-1", "title", "explicit"].map(str::to_owned))
        );
        assert_eq!(decode("a%20b%23c").unwrap(), "a b#c");
        for malformed in ["%", "%2", "%xx", "%ff"] {
            assert!(decode(malformed).is_err());
        }
        assert_eq!(
            prose("```\n```not-a-close\n# hidden\n```\n# visible"),
            vec!["", "", "", "", "# visible"]
        );
    }

    #[test]
    fn inline_links_require_closure_and_explicit_complex_paths() {
        assert_eq!(
            inline_target("guide.md#heading) after").unwrap(),
            "guide.md#heading"
        );
        assert_eq!(
            inline_target("<some (file).md>) after").unwrap(),
            "some (file).md"
        );
        for malformed in [
            "guide.md",
            "<guide.md",
            "<guide.md>",
            "some file.md)",
            "file(name).md)",
            "guide.md \"title\")",
        ] {
            assert!(inline_target(malformed).is_err(), "{malformed}");
        }
    }
}
