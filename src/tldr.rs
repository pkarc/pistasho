//! Parser de páginas tldr (formato markdown de tldr-pages).
//!
//! Sin dependencias externas para poder probarlo con `rustc --test`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub name: String,
    pub summary: String,
    pub examples: Vec<Example>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Example {
    pub desc: String,
    pub cmd: String,
}

/// Ubicación de una página dentro del zip de tldr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PagePath {
    /// `en`, `es`, `pt_BR`…
    pub lang: String,
    /// `common`, `linux`, `osx`…
    pub platform: String,
    pub name: String,
}

/// Interpreta rutas como `pages/common/tar.md` o `tldr/pages.es/linux/ss.md`.
pub fn page_path(path: &str) -> Option<PagePath> {
    let comps: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
    let i = comps
        .iter()
        .position(|c| *c == "pages" || c.starts_with("pages."))?;
    if comps.len() != i + 3 {
        return None;
    }
    let lang = match comps[i].strip_prefix("pages.") {
        Some(l) => l.to_string(),
        None => "en".to_string(),
    };
    let name = comps[i + 2].strip_suffix(".md")?.to_string();
    Some(PagePath {
        lang,
        platform: comps[i + 1].to_string(),
        name,
    })
}

pub fn parse(md: &str) -> Page {
    let mut name = String::new();
    let mut summary = Vec::new();
    let mut examples = Vec::new();
    let mut pending_desc: Option<String> = None;

    for line in md.lines() {
        let line = line.trim();
        if let Some(t) = line.strip_prefix("# ") {
            if name.is_empty() {
                name = t.trim().to_string();
            }
        } else if let Some(t) = line.strip_prefix('>') {
            let t = t.trim();
            // la línea "More information: <url>" (o su traducción) no aporta a la búsqueda
            if !t.contains("://") && !t.is_empty() {
                summary.push(t.to_string());
            }
        } else if let Some(t) = line.strip_prefix("- ") {
            pending_desc = Some(clean_desc(t));
        } else if line.starts_with('`') && line.ends_with('`') && line.len() >= 2 {
            if let Some(desc) = pending_desc.take() {
                let cmd = line[1..line.len() - 1].trim().to_string();
                if !cmd.is_empty() {
                    examples.push(Example { desc, cmd });
                }
            }
        }
    }

    Page {
        name,
        summary: summary.join(" "),
        examples,
    }
}

fn clean_desc(s: &str) -> String {
    let s = s.trim().trim_end_matches(':').trim();
    // tldr marca algunas palabras como [l]ist; quitamos los corchetes de una letra
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' && i + 2 < chars.len() && chars[i + 2] == ']' && chars[i + 1].is_alphanumeric() {
            out.push(chars[i + 1]);
            i += 3;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAR: &str = "# tar

> Archiving utility.
> Often combined with a compression method, such as `gzip` or `bzip2`.
> More information: <https://www.gnu.org/software/tar>.

- [c]reate an archive and write it to a [f]ile:

`tar cf {{path/to/target.tar}} {{path/to/file1 path/to/file2 ...}}`

- E[x]tract a (compressed) archive [f]ile into the current directory [v]erbosely:

`tar xvf {{path/to/source.tar[.gz|.bz2|.xz]}}`
";

    #[test]
    fn parses_page() {
        let p = parse(TAR);
        assert_eq!(p.name, "tar");
        assert!(p.summary.starts_with("Archiving utility."));
        assert!(!p.summary.contains("More information"));
        assert_eq!(p.examples.len(), 2);
        assert_eq!(p.examples[0].desc, "create an archive and write it to a file");
        assert_eq!(
            p.examples[0].cmd,
            "tar cf {{path/to/target.tar}} {{path/to/file1 path/to/file2 ...}}"
        );
        assert_eq!(
            p.examples[1].desc,
            "Extract a (compressed) archive file into the current directory verbosely"
        );
    }

    #[test]
    fn paths() {
        assert_eq!(
            page_path("pages/common/tar.md"),
            Some(PagePath {
                lang: "en".into(),
                platform: "common".into(),
                name: "tar".into()
            })
        );
        assert_eq!(
            page_path("tldr/pages.es/linux/ss.md"),
            Some(PagePath {
                lang: "es".into(),
                platform: "linux".into(),
                name: "ss".into()
            })
        );
        assert_eq!(page_path("pages/common/"), None);
        assert_eq!(page_path("LICENSE.md"), None);
        assert_eq!(page_path("pages/index.json"), None);
    }

    #[test]
    fn spanish_page() {
        let md = "# ss\n\n> Herramienta para investigar sockets.\n> Más información: <https://manned.org/ss.8>.\n\n- Muestra todos los sockets TCP en escucha:\n\n`ss {{[-lt|--listening --tcp]}}`\n";
        let p = parse(md);
        assert_eq!(p.summary, "Herramienta para investigar sockets.");
        assert_eq!(p.examples[0].desc, "Muestra todos los sockets TCP en escucha");
    }
}
