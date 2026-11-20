//! Lectura del historial de zsh (`~/.zsh_history`).
//!
//! Sin dependencias externas para poder probarlo con `rustc --test`.

use std::collections::HashMap;

/// Un comando del historial con cuántas veces se usó y su posición más reciente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistItem {
    pub cmd: String,
    pub count: u32,
    /// Índice de la última aparición; mayor = más reciente.
    pub last: usize,
}

/// zsh "metafica" los bytes especiales: 0x83 seguido de `b` significa `b ^ 0x20`.
pub fn unmetafy(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut it = bytes.iter();
    while let Some(&b) = it.next() {
        if b == 0x83 {
            if let Some(&n) = it.next() {
                out.push(n ^ 0x20);
            }
        } else {
            out.push(b);
        }
    }
    out
}

/// Separa el historial en comandos. Soporta el formato extendido
/// (`: 1700000000:0;cmd`) y comandos multilínea terminados en `\`.
pub fn parse_lines(text: &str) -> Vec<String> {
    let mut cmds = Vec::new();
    let mut current: Option<String> = None;

    for line in text.lines() {
        let piece = match current.take() {
            Some(mut acc) => {
                acc.push('\n');
                acc.push_str(line);
                acc
            }
            None => strip_extended(line).to_string(),
        };
        if piece.ends_with('\\') && !piece.ends_with("\\\\") {
            let mut p = piece;
            p.pop();
            current = Some(p);
        } else {
            cmds.push(piece);
        }
    }
    if let Some(p) = current {
        cmds.push(p);
    }
    cmds.into_iter()
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .collect()
}

fn strip_extended(line: &str) -> &str {
    if let Some(rest) = line.strip_prefix(": ") {
        if let Some(pos) = rest.find(';') {
            let meta = &rest[..pos];
            if meta.chars().all(|c| c.is_ascii_digit() || c == ':') {
                return &rest[pos + 1..];
            }
        }
    }
    line
}

/// Comandos que no vale la pena sugerir.
fn is_noise(cmd: &str) -> bool {
    const NOISE: &[&str] = &[
        "ls", "ll", "la", "cd", "clear", "exit", "pwd", "history", "fg", "bg",
    ];
    let first = cmd.split_whitespace().next().unwrap_or("");
    (NOISE.contains(&first) && cmd.split_whitespace().count() <= 2) || cmd.len() < 3
}

/// Agrupa comandos repetidos y devuelve los `limit` más recientes.
pub fn aggregate(cmds: &[String], limit: usize) -> Vec<HistItem> {
    let mut map: HashMap<&str, (u32, usize)> = HashMap::new();
    for (i, c) in cmds.iter().enumerate() {
        if is_noise(c) {
            continue;
        }
        let e = map.entry(c.as_str()).or_insert((0, 0));
        e.0 += 1;
        e.1 = i;
    }
    let mut items: Vec<HistItem> = map
        .into_iter()
        .map(|(cmd, (count, last))| HistItem {
            cmd: cmd.to_string(),
            count,
            last,
        })
        .collect();
    items.sort_by(|a, b| b.last.cmp(&a.last));
    items.truncate(limit);
    items
}

/// Lee y procesa un archivo de historial completo.
pub fn load(bytes: &[u8], limit: usize) -> Vec<HistItem> {
    let text = String::from_utf8_lossy(&unmetafy(bytes)).into_owned();
    aggregate(&parse_lines(&text), limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extended_and_plain() {
        let h = ": 1700000000:0;git status\n: 1700000001:3;cargo build --release\nls -la /tmp\n";
        assert_eq!(
            parse_lines(h),
            vec!["git status", "cargo build --release", "ls -la /tmp"]
        );
    }

    #[test]
    fn multiline() {
        let h = ": 1700000000:0;docker run \\\n  -it ubuntu\n: 1700000002:0;echo hola\n";
        assert_eq!(parse_lines(h), vec!["docker run \n  -it ubuntu", "echo hola"]);
    }

    #[test]
    fn metafied_utf8() {
        // regla de zsh: 0x83 seguido de b significa b ^ 0x20 (0x83 0xA1 -> 0x81)
        assert_eq!(unmetafy(&[b'a', 0x83, 0xA1, b'b']), vec![b'a', 0x81, b'b']);
    }

    #[test]
    fn aggregate_counts_and_recency() {
        let cmds: Vec<String> = ["ssh root@srv", "ls", "cargo test", "ssh root@srv", "cd /tmp"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let items = aggregate(&cmds, 10);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].cmd, "ssh root@srv");
        assert_eq!(items[0].count, 2);
        assert_eq!(items[1].cmd, "cargo test");
    }
}
