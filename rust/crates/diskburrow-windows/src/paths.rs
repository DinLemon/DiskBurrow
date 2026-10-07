pub fn equals_path(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}
pub fn is_within(path: &str, root: &str) -> bool {
    equals_path(path, root)
        || path
            .to_lowercase()
            .starts_with(&format!("{}\\", root.trim_end_matches('\\').to_lowercase()))
}

/// Local drive paths only: rejects UNC/device paths, streams, DOS aliases, trailing dots/spaces.
/// Lexical normalization does not open or follow the filesystem.
pub fn normalize_local_path(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    if bytes.len() < 3
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1] != b':'
        || !matches!(bytes[2], b'\\' | b'/')
    {
        return None;
    }
    let rest = path[3..].replace('/', "\\");
    let mut parts = Vec::new();
    for segment in rest.split('\\') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            parts.pop()?;
            continue;
        }
        if segment.ends_with(['.', ' '])
            || segment.chars().any(|c| c < ' ' || "<>:\"|?*".contains(c))
        {
            return None;
        }
        let stem = segment.split('.').next()?.to_ascii_uppercase();
        if ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&stem.as_str())
            || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.len() == 4
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        {
            return None;
        }
        parts.push(segment);
    }
    Some(format!(
        "{}:\\{}",
        (bytes[0] as char).to_ascii_uppercase(),
        parts.join("\\")
    ))
}
pub(crate) fn canonical(path: &str) -> Option<String> {
    let normalized = normalize_local_path(path)?;
    let supplied = if path.len() > 3 {
        path.trim_end_matches('\\')
    } else {
        path
    };
    equals_path(supplied, &normalized).then_some(normalized)
}
pub(crate) fn parent(path: &str) -> Option<String> {
    if path.len() <= 3 {
        return None;
    }
    let pos = path.rfind('\\')?;
    Some(if pos == 2 {
        path[..3].to_owned()
    } else {
        path[..pos].to_owned()
    })
}
pub(crate) fn chain(path: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut current = Some(path.to_owned());
    while let Some(p) = current {
        current = parent(&p);
        values.push(p);
    }
    values.reverse();
    values
}
