//! Closed command grammar for the read-only elevated child.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelperArgs {
    pub pipe: String,
    pub nonce: String,
    pub root: String,
    pub parent: u32,
}
pub fn parse(args: &[String]) -> Result<Option<HelperArgs>, String> {
    if !args
        .iter()
        .take_while(|arg| arg.as_str() != "--")
        .any(|a| a == "--mft-helper")
    {
        return Ok(None);
    }
    if args.len() != 9 || args[0] != "--mft-helper" {
        return Err("Invalid helper command".into());
    }
    let mut values = std::collections::HashMap::new();
    for pair in args[1..].chunks_exact(2) {
        if !["--pipe", "--nonce", "--root", "--parent"].contains(&pair[0].as_str())
            || values.insert(pair[0].as_str(), pair[1].as_str()).is_some()
        {
            return Err("Unknown or duplicate helper argument".into());
        }
    }
    let get = |key| {
        values
            .get(key)
            .copied()
            .ok_or_else(|| "Missing helper argument".to_owned())
    };
    let parent = get("--parent")?
        .parse::<u32>()
        .map_err(|_| "Invalid helper parent")?;
    if parent == 0 {
        return Err("Invalid helper parent".into());
    }
    let pipe = get("--pipe")?;
    let prefix = format!("DiskBurrowRustMft-{parent}-");
    if !pipe
        .strip_prefix(&prefix)
        .is_some_and(|suffix| suffix.len() == 32 && suffix.bytes().all(|c| c.is_ascii_hexdigit()))
    {
        return Err("Invalid private pipe name".into());
    }
    let nonce = get("--nonce")?;
    if nonce.len() != 64 || !nonce.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("Invalid helper nonce".into());
    }
    let root = get("--root")?;
    let bytes = root.as_bytes();
    if bytes.len() != 3 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' || bytes[2] != b'\\'
    {
        return Err("The helper can read only a whole local volume".into());
    }
    Ok(Some(HelperArgs {
        pipe: pipe.into(),
        nonce: nonce.into(),
        root: root.into(),
        parent,
    }))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn valid() -> Vec<String> {
        vec![
            "--mft-helper".into(),
            "--pipe".into(),
            format!("DiskBurrowRustMft-42-{}", "a".repeat(32)),
            "--nonce".into(),
            "b".repeat(64),
            "--root".into(),
            r"C:\".into(),
            "--parent".into(),
            "42".into(),
        ]
    }
    #[test]
    fn positional_separator_does_not_activate_the_elevated_helper() {
        assert_eq!(parse(&["--".into(), "--mft-helper".into()]).unwrap(), None);
        let mut args = valid();
        args.push("--".into());
        assert!(parse(&args).is_err(), "Real helper grammar remains exact");
    }
    #[test]
    fn only_exact_local_volume_and_authenticated_parent_are_accepted() {
        assert!(parse(&["--background".into()]).unwrap().is_none());
        let args = parse(&valid()).unwrap().unwrap();
        assert_eq!(args.parent, 42);
        assert_eq!(args.root, r"C:\");
        for root in [
            r"C:\Windows",
            r"\\server\share",
            r"\\.\C:",
            "C:/",
            "relative",
            "C:\\ --anything",
        ] {
            let mut args = valid();
            args[6] = root.into();
            assert!(parse(&args).is_err());
        }
    }
    #[test]
    fn duplicate_unknown_missing_and_control_arguments_are_rejected() {
        for changed in 0..6 {
            let mut args = valid();
            match changed {
                0 => args.push("--delete".into()),
                1 => args[2] = r"..\other".into(),
                2 => args[4] = "a".repeat(63),
                3 => args[8] = "0".into(),
                4 => args[8] = "43".into(),
                _ => args[1] = "--root".into(),
            }
            assert!(parse(&args).is_err());
        }
        let mut args = valid();
        args.pop();
        assert!(parse(&args).is_err());
    }
}
