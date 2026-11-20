//! Marcadores `{{nombre}}` dentro de un comando.
//!
//! Sin dependencias externas para poder probarlo con `rustc --test`.

/// Un trozo de plantilla: texto literal o un marcador.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    Text(String),
    Placeholder(String),
}

/// Divide un comando en texto y marcadores. Un `{{` sin cierre se trata como texto.
pub fn parse(cmd: &str) -> Vec<Part> {
    let mut parts = Vec::new();
    let mut rest = cmd;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        match after.find("}}") {
            Some(end) => {
                if start > 0 {
                    push_text(&mut parts, &rest[..start]);
                }
                parts.push(Part::Placeholder(after[..end].to_string()));
                rest = &after[end + 2..];
            }
            None => break,
        }
    }
    if !rest.is_empty() {
        push_text(&mut parts, rest);
    }
    parts
}

fn push_text(parts: &mut Vec<Part>, s: &str) {
    if let Some(Part::Text(prev)) = parts.last_mut() {
        prev.push_str(s);
    } else {
        parts.push(Part::Text(s.to_string()));
    }
}

/// Nombres de los marcadores en orden de aparición.
pub fn names(cmd: &str) -> Vec<String> {
    parse(cmd)
        .into_iter()
        .filter_map(|p| match p {
            Part::Placeholder(n) => Some(n),
            Part::Text(_) => None,
        })
        .collect()
}

pub fn has_placeholders(cmd: &str) -> bool {
    parse(cmd).iter().any(|p| matches!(p, Part::Placeholder(_)))
}

/// Reconstruye el comando sustituyendo marcadores en orden.
/// `None` (o un valor vacío) deja el marcador original `{{x}}`.
pub fn fill(cmd: &str, values: &[Option<String>]) -> String {
    let mut out = String::new();
    let mut i = 0;
    for part in parse(cmd) {
        match part {
            Part::Text(t) => out.push_str(&t),
            Part::Placeholder(name) => {
                match values.get(i).and_then(|v| v.as_deref()).filter(|v| !v.is_empty()) {
                    Some(v) => out.push_str(v),
                    None => {
                        out.push_str("{{");
                        out.push_str(&name);
                        out.push_str("}}");
                    }
                }
                i += 1;
            }
        }
    }
    out
}

/// tldr usa `{{[-o|--opcion]}}` para opciones con forma corta y larga.
/// Las convertimos en la forma larga, que es más legible en el prompt.
pub fn normalize_tldr(cmd: &str) -> String {
    let mut out = String::new();
    for part in parse(cmd) {
        match part {
            Part::Text(t) => out.push_str(&t),
            Part::Placeholder(inner) => {
                if let Some(long) = option_long_form(&inner) {
                    out.push_str(&long);
                } else {
                    out.push_str("{{");
                    out.push_str(&inner);
                    out.push_str("}}");
                }
            }
        }
    }
    out
}

fn option_long_form(inner: &str) -> Option<String> {
    let body = inner.strip_prefix('[')?.strip_suffix(']')?;
    let alts: Vec<&str> = body.split('|').collect();
    if alts.len() < 2 || !alts.iter().all(|a| a.starts_with('-')) {
        return None;
    }
    Some(alts.last().unwrap().to_string())
}

/// Nombre corto para mostrar en el prompt de relleno: `path/to/file` → `file`.
pub fn display_name(name: &str) -> &str {
    let n = name.trim_end_matches("...").trim();
    n.rsplit('/').next().unwrap_or(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic() {
        assert_eq!(
            parse("tar -czf {{archivo}}.tar.gz {{dir}}"),
            vec![
                Part::Text("tar -czf ".into()),
                Part::Placeholder("archivo".into()),
                Part::Text(".tar.gz ".into()),
                Part::Placeholder("dir".into()),
            ]
        );
    }

    #[test]
    fn unclosed_is_text() {
        assert_eq!(parse("echo {{oops"), vec![Part::Text("echo {{oops".into())]);
        assert!(!has_placeholders("echo {{oops"));
    }

    #[test]
    fn fill_partial() {
        let cmd = "lsof -i :{{puerto}} -u {{usuario}}";
        assert_eq!(names(cmd), vec!["puerto", "usuario"]);
        assert_eq!(
            fill(cmd, &[Some("8080".into()), None]),
            "lsof -i :8080 -u {{usuario}}"
        );
        assert_eq!(fill(cmd, &[Some("".into())]), cmd);
    }

    #[test]
    fn tldr_options() {
        assert_eq!(
            normalize_tldr("ls {{[-a|--all]}} {{path/to/dir}}"),
            "ls --all {{path/to/dir}}"
        );
        // un marcador normal entre corchetes no se toca
        assert_eq!(normalize_tldr("echo {{[texto]}}"), "echo {{[texto]}}");
    }

    #[test]
    fn display() {
        assert_eq!(display_name("path/to/file"), "file");
        assert_eq!(display_name("path/to/file1 path/to/file2 ..."), "file2");
        assert_eq!(display_name("puerto"), "puerto");
    }
}
