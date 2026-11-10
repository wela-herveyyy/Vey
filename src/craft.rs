//! ArtCraft / Storytold native Rust app suite integration for the Apps tab.
//! Detects existing installations and downloads portable Windows releases from GitHub
//! on a background worker thread over WinHTTP (zero external crates), posting `WM_CRAFT`.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use crate::gfx::Color;
use crate::store;
use crate::work::{parse_json, Json};

/// Posted to the main window whenever Craft app scan or download progress changes.
pub const WM_CRAFT: u32 = 0x8000 + 14; // WM_APP + 14

pub const CRAFT_COUNT: usize = 4;

pub struct Spec {
    pub slug: &'static str,
    pub name: &'static str,
    pub tagline: &'static str,
    pub repo: &'static str,
    pub web_url: &'static str,
    pub glyph: char,
    pub tint: Color,
    pub exes: &'static [&'static str],
    pub win_dirs: &'static [&'static str],
}

pub const SPECS: [Spec; CRAFT_COUNT] = [
    Spec {
        slug: "craft-launcher",
        name: "Craft Launcher",
        tagline: "ArtCraft app launcher",
        repo: "storytold/craft-launcher",
        web_url: "https://github.com/storytold/craft-launcher",
        glyph: '\u{E71D}',
        tint: Color(0.98, 0.70, 0.20, 1.0), // Amber (C_MEM)
        exes: &["artcraft-launcher.exe", "craft-launcher.exe"],
        win_dirs: &["ArtCraft Launcher", "Craft Launcher", "craft-launcher"],
    },
    Spec {
        slug: "gridcraft",
        name: "GridCraft",
        tagline: "Native spreadsheets",
        repo: "storytold/gridcraft",
        web_url: "https://getartcraft.com/apps/gridcraft",
        glyph: '\u{E80A}',
        tint: Color(0.40, 0.85, 0.50, 1.0), // Sage (C_BAT)
        exes: &["gridcraft.exe"],
        win_dirs: &["GridCraft", "gridcraft"],
    },
    Spec {
        slug: "deckcraft",
        name: "DeckCraft",
        tagline: "Native presentations",
        repo: "storytold/deckcraft",
        web_url: "https://github.com/storytold/deckcraft",
        glyph: '\u{E786}',
        tint: Color(0.98, 0.45, 0.55, 1.0), // Coral / Rose
        exes: &["deckcraft.exe"],
        win_dirs: &["DeckCraft", "deckcraft"],
    },
    Spec {
        slug: "wordcraft",
        name: "WordCraft",
        tagline: "Native word processor",
        repo: "storytold/wordcraft",
        web_url: "https://getartcraft.com/apps/wordcraft",
        glyph: '\u{E8A5}',
        tint: Color(0.30, 0.78, 0.98, 1.0), // Sky (C_CPU)
        exes: &["wordcraft.exe"],
        win_dirs: &["WordCraft", "wordcraft"],
    },
];

pub fn find_craft_cli(slug: &str) -> Option<PathBuf> {
    let cli_name = match slug {
        "cadcraft" | "CADCraft" => "cadcraft-cli.exe",
        "wordcraft" | "WordCraft" => "wordcraft-cli.exe",
        "gridcraft" | "GridCraft" => "gridcraft-cli.exe",
        "deckcraft" | "DeckCraft" => "deckcraft-cli.exe",
        _ => return None,
    };

    // 1. Program Files
    let pf_dir = match slug {
        "cadcraft" | "CADCraft" => "CADCraft",
        "wordcraft" | "WordCraft" => "WordCraft",
        "gridcraft" | "GridCraft" => "GridCraft",
        "deckcraft" | "DeckCraft" => "DeckCraft",
        _ => "",
    };
    if let Ok(pf) = std::env::var("ProgramFiles") {
        let p = PathBuf::from(pf).join(pf_dir).join(cli_name);
        if p.is_file() {
            return Some(p);
        }
    }

    // 2. Vey craft managed directory
    let craft_root = store::data_dir().join("craft");
    let managed = craft_root.join(slug.to_ascii_lowercase());
    if let Some(exe) = find_exe_in_tree(&managed, &[cli_name], 3) {
        return Some(exe);
    }

    // 3. User cargo bin directory
    if let Ok(prof) = std::env::var("USERPROFILE") {
        let p = PathBuf::from(prof).join(".cargo").join("bin").join(cli_name);
        if p.is_file() {
            return Some(p);
        }
    }

    None
}

pub fn list_available_craft_clis() -> Vec<(&'static str, PathBuf)> {
    let mut out = Vec::new();
    for slug in ["cadcraft", "wordcraft", "gridcraft", "deckcraft"] {
        if let Some(path) = find_craft_cli(slug) {
            out.push((slug, path));
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct McpToolInfo {
    pub name: String,
    pub description: String,
}

/// Optional port configuration from `config.ini` (`wordcraft_port=`, `gridcraft_port=`, etc.).
pub fn get_craft_port(slug: &str) -> Option<u16> {
    let ini = std::fs::read_to_string(store::data_dir().join("config.ini")).ok()?;
    let key = format!("{}_port=", slug.to_ascii_lowercase());
    for line in ini.lines() {
        let trimmed = line.trim();
        if let Some(val) = trimmed.strip_prefix(&key) {
            if let Ok(p) = val.trim().parse::<u16>() {
                if p > 0 {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// Dynamically writes or updates Vey's own MCP configuration file (`%APPDATA%\vey\mcp.json` and `mcp_config.json`).
/// Uses detected paths on the system so nothing is hardcoded.
pub fn sync_vey_mcp_config() -> Option<PathBuf> {
    let clis = list_available_craft_clis();
    if clis.is_empty() {
        return None;
    }

    let mut server_entries = Vec::new();
    for (slug, path) in &clis {
        let p_str = path.to_string_lossy();
        let port_arg = if let Some(p) = get_craft_port(slug) {
            format!(r#", "--connect", "{}""#, p)
        } else {
            String::new()
        };
        let entry = format!(
            "    \"{}\": {{\n      \"command\": {},\n      \"args\": [\"mcp\"{}]\n    }}",
            slug,
            crate::work::json_str(&p_str),
            port_arg
        );
        server_entries.push(entry);
    }

    let json_content = format!(
        "{{\n  \"mcpServers\": {{\n{}\n  }}\n}}\n",
        server_entries.join(",\n")
    );

    let vdir = store::data_dir();
    let _ = std::fs::create_dir_all(&vdir);
    let mcp_json = vdir.join("mcp.json");
    let mcp_cfg = vdir.join("mcp_config.json");
    let _ = std::fs::write(&mcp_json, &json_content);
    let _ = std::fs::write(&mcp_cfg, &json_content);

    Some(mcp_json)
}

/// Dynamically queries `tools/list` over MCP on stdio (or `--connect <port>`).
/// Retrieves the full real-time tool catalog directly from the ArtCraft application.
pub fn fetch_craft_mcp_tools(slug: &str, port: Option<u16>) -> Result<Vec<McpToolInfo>, String> {
    #[cfg(windows)]
    {
        use std::io::{BufRead, BufReader, Write};
        use std::os::windows::process::CommandExt;

        let cli = find_craft_cli(slug).ok_or_else(|| format!("{slug} CLI not found"))?;
        let mut cmd = std::process::Command::new(&cli);
        cmd.arg("mcp");
        if let Some(p) = port.or_else(|| get_craft_port(slug)) {
            cmd.arg("--connect").arg(format!("{p}"));
        }
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .creation_flags(0x08000000); // CREATE_NO_WINDOW

        let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn {slug} mcp: {e}"))?;
        let mut stdin = child.stdin.take().ok_or("No stdin")?;
        let mut reader = BufReader::new(child.stdout.take().ok_or("No stdout")?);

        // 1. Handshake initialize
        let init_msg = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"vey","version":"0.13"}}}"#;
        stdin.write_all(init_msg.as_bytes()).map_err(|e| e.to_string())?;
        stdin.write_all(b"\n").map_err(|e| e.to_string())?;
        stdin.flush().map_err(|e| e.to_string())?;

        let mut line = String::new();
        reader.read_line(&mut line).map_err(|e| e.to_string())?;

        // 2. Query tools/list
        let list_msg = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#;
        stdin.write_all(list_msg.as_bytes()).map_err(|e| e.to_string())?;
        stdin.write_all(b"\n").map_err(|e| e.to_string())?;
        stdin.flush().map_err(|e| e.to_string())?;

        let mut resp = String::new();
        reader.read_line(&mut resp).map_err(|e| e.to_string())?;

        let _ = child.kill();

        // 3. Parse tools array from json response
        let root = crate::work::parse_json(&resp).ok_or("Failed to parse tools/list JSON")?;
        let result = root.get("result").ok_or("No result in tools/list response")?;
        let tools_arr = result.get("tools").and_then(|t| t.as_arr()).ok_or("No tools array")?;

        let mut out = Vec::new();
        for t in tools_arr {
            let name = t.str_or("name").to_string();
            let desc = t.str_or("description").to_string();
            if !name.is_empty() {
                out.push(McpToolInfo {
                    name,
                    description: desc,
                });
            }
        }
        Ok(out)
    }
    #[cfg(not(windows))]
    {
        Err("MCP tools fetch only supported on Windows".into())
    }
}

/// Formats a complete summary of all ArtCraft MCP tools discovered dynamically
/// from the live CLI MCP servers to provide to the AI model.
pub fn get_all_craft_mcp_tools_summary() -> String {
    let clis = list_available_craft_clis();
    if clis.is_empty() {
        return "No Craft MCP servers detected.".to_string();
    }
    let mut sections = Vec::new();
    for (slug, path) in clis {
        let app_title = match slug {
            "wordcraft" => "WordCraft",
            "gridcraft" => "GridCraft",
            "deckcraft" => "DeckCraft",
            "cadcraft" => "CADCraft",
            other => other,
        };
        let mut tool_lines = Vec::new();
        if let Ok(tools) = fetch_craft_mcp_tools(slug, None) {
            for t in tools {
                let first_sentence = t.description.split('.').next().unwrap_or(&t.description).trim();
                tool_lines.push(format!("  - `{}`: {}", t.name, first_sentence));
            }
        } else {
            // Fallback to core known tools if server is busy
            match slug {
                "wordcraft" => {
                    tool_lines.push("  - `new_document`, `open_document`, `save_document`, `type_text`, `insert_table`, `execute`, `batch`, `inspect_document`".into());
                }
                "gridcraft" => {
                    tool_lines.push("  - `new_workbook`, `open_workbook`, `save_workbook`, `write_range`, `read_range`, `create_table`, `insert_chart`, `execute_command`".into());
                }
                "deckcraft" => {
                    tool_lines.push("  - `new_presentation`, `open_presentation`, `save_presentation`, `add_slide`, `set_text`, `add_shape`, `insert_table`, `run_command`".into());
                }
                "cadcraft" => {
                    tool_lines.push("  - `new_drawing`, `open`, `save`, `command_line`, `render`, `execute`".into());
                }
                _ => {}
            }
        }
        sections.push(format!(
            "### {} MCP Server (`{}`):\n{}",
            app_title,
            path.display(),
            tool_lines.join("\n")
        ));
    }
    sections.join("\n\n")
}

pub fn find_craft_gui(slug: &str) -> Option<PathBuf> {
    let exes = match slug {
        "cadcraft" | "CADCraft" => &["cadcraft.exe"][..],
        "wordcraft" | "WordCraft" => &["wordcraft.exe"][..],
        "gridcraft" | "GridCraft" => &["gridcraft.exe"][..],
        "deckcraft" | "DeckCraft" => &["deckcraft.exe"][..],
        _ => return None,
    };
    let pf_dir = match slug {
        "cadcraft" | "CADCraft" => "CADCraft",
        "wordcraft" | "WordCraft" => "WordCraft",
        "gridcraft" | "GridCraft" => "GridCraft",
        "deckcraft" | "DeckCraft" => "DeckCraft",
        _ => "",
    };
    if let Ok(pf) = std::env::var("ProgramFiles") {
        for exe in exes {
            let p = PathBuf::from(&pf).join(pf_dir).join(exe);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    let craft_root = store::data_dir().join("craft");
    let managed = craft_root.join(slug.to_ascii_lowercase());
    if let Some(exe) = find_exe_in_tree(&managed, exes, 3) {
        return Some(exe);
    }
    if let Ok(prof) = std::env::var("USERPROFILE") {
        for exe in exes {
            let p = PathBuf::from(&prof).join(".cargo").join("bin").join(exe);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

pub fn vey_docs_dir() -> PathBuf {
    let p = if let Ok(prof) = std::env::var("USERPROFILE") {
        PathBuf::from(prof).join("Documents").join("Vey")
    } else {
        store::data_dir().join("documents")
    };
    let _ = std::fs::create_dir_all(&p);
    p
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatDocMode {
    EditExisting,
    CreateNew,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveCraftDoc {
    pub slug: String,
    pub app_name: String,
    pub file_name: String,
    pub file_path: PathBuf,
}

pub fn detect_craft_doc_from_path(path: &Path) -> Option<ActiveCraftDoc> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let (slug, app_name) = match ext.as_str() {
        "docx" | "doc" => ("wordcraft", "WordCraft"),
        "xlsx" | "xls" | "csv" => ("gridcraft", "GridCraft"),
        "deckcraft" | "pptx" | "ppt" => ("deckcraft", "DeckCraft"),
        "dxf" | "dwg" => ("cadcraft", "CADCraft"),
        _ => return None,
    };
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Document")
        .to_string();
    Some(ActiveCraftDoc {
        slug: slug.to_string(),
        app_name: app_name.to_string(),
        file_name,
        file_path: path.to_path_buf(),
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct CraftDocCard {
    pub slug: String,
    pub app_name: String,
    pub file_name: String,
    pub file_path: PathBuf,
    pub size_label: String,
}

pub fn ensure_craft_installed(slug: &str) -> Result<(PathBuf, PathBuf), String> {
    let lower = slug.to_ascii_lowercase();
    let norm = match lower.as_str() {
        "wordcraft" | "word" | "docx" => "wordcraft",
        "gridcraft" | "grid" | "xlsx" | "excel" | "csv" => "gridcraft",
        "deckcraft" | "deck" | "pptx" | "slides" | "presentation" => "deckcraft",
        "cadcraft" | "cad" | "dxf" => "cadcraft",
        other => other,
    };
    if let Some(cli) = find_craft_cli(norm) {
        let gui = find_craft_gui(norm).unwrap_or_else(|| cli.clone());
        return Ok((cli, gui));
    }

    let idx = match norm {
        "gridcraft" | "GridCraft" => 1,
        "deckcraft" | "DeckCraft" => 2,
        "wordcraft" | "WordCraft" => 3,
        _ => return Err(format!("Automated installation not configured for {slug}")),
    };

    #[cfg(windows)]
    {
        let client = http::Client::new();
        let dummy_shared = Arc::new(Mutex::new(CraftData::default()));
        do_install(&client, 0, &dummy_shared, idx);
        if let Some(cli) = find_craft_cli(norm) {
            let gui = find_craft_gui(norm).unwrap_or_else(|| cli.clone());
            return Ok((cli, gui));
        }
    }
    Err(format!("Could not locate or install {slug}"))
}

#[cfg(windows)]
pub fn execute_craft_mcp_flow(
    norm: &str,
    cli: &Path,
    out_path: &Path,
    target_override: Option<&Path>,
    clean_content: &str,
) -> Result<(), String> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::windows::process::CommandExt;

    let mut cmd = std::process::Command::new(cli);
    cmd.arg("mcp");
    if let Some(port) = get_craft_port(norm) {
        cmd.arg("--connect").arg(port.to_string());
    }
    let mut child = cmd
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .spawn()
        .map_err(|e| format!("Failed to spawn mcp: {e}"))?;

    let mut stdin = child.stdin.take().ok_or("No stdin")?;
    let mut reader = BufReader::new(child.stdout.take().ok_or("No stdout")?);

    // 1. Handshake
    let init_msg = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"vey","version":"0.13"}}}"#;
    stdin.write_all(init_msg.as_bytes()).map_err(|e| e.to_string())?;
    stdin.write_all(b"\n").map_err(|e| e.to_string())?;
    stdin.flush().map_err(|e| e.to_string())?;

    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| e.to_string())?;
    if !line.contains(r#""result""#) {
        let _ = child.kill();
        return Err("MCP initialize failed".into());
    }

    let out_path_str = out_path.to_string_lossy();
    let out_json_str = crate::work::json_str(&out_path_str);

    let mut call_id = 2u64;
    let mut send_call = |name: &str, args_json: &str| -> Result<String, String> {
        let msg = format!(
            r#"{{"jsonrpc":"2.0","id":{call_id},"method":"tools/call","params":{{"name":"{name}","arguments":{args_json}}}}}"#
        );
        call_id += 1;
        stdin.write_all(msg.as_bytes()).map_err(|e| e.to_string())?;
        stdin.write_all(b"\n").map_err(|e| e.to_string())?;
        stdin.flush().map_err(|e| e.to_string())?;

        let mut res = String::new();
        reader.read_line(&mut res).map_err(|e| e.to_string())?;
        Ok(res)
    };

    match norm {
        "wordcraft" => {
            if let Some(target) = target_override {
                let p_str = target.to_string_lossy();
                let _ = send_call("open_document", &format!(r#"{{"path":{}}}"#, crate::work::json_str(&p_str)));
            } else {
                let _ = send_call("new_document", "{}");
            }
            let _ = send_call(
                "type_text",
                &format!(r#"{{"text":{}}}"#, crate::work::json_str(clean_content)),
            );
            let save_res = send_call("save_document", &format!(r#"{{"path":{out_json_str}}}"#))?;
            if save_res.contains(r#""isError":true"#) {
                let _ = child.kill();
                return Err("WordCraft save failed".into());
            }
        }
        "gridcraft" => {
            if let Some(target) = target_override {
                let p_str = target.to_string_lossy();
                let _ = send_call("open_workbook", &format!(r#"{{"path":{}}}"#, crate::work::json_str(&p_str)));
            } else {
                let _ = send_call("new_workbook", "{}");
            }
            let mut rows = Vec::new();
            for l in clean_content.lines() {
                let trimmed = l.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let cells: Vec<String> = trimmed
                    .split(',')
                    .map(|c| crate::work::json_str(c.trim()))
                    .collect();
                if !cells.is_empty() {
                    rows.push(format!("[{}]", cells.join(",")));
                }
            }
            if rows.is_empty() {
                rows.push(r#"["Data","Values"]"#.to_string());
            }
            let values_json = format!("[{}]", rows.join(","));
            let _ = send_call(
                "write_range",
                &format!(r#"{{"range":"A1","values":{values_json}}}"#),
            );
            let save_res = send_call("save_workbook", &format!(r#"{{"path":{out_json_str}}}"#))?;
            if save_res.contains(r#""isError":true"#) {
                let _ = child.kill();
                return Err("GridCraft save failed".into());
            }
        }
        "deckcraft" => {
            if let Some(target) = target_override {
                let p_str = target.to_string_lossy();
                let _ = send_call("open_presentation", &format!(r#"{{"path":{}}}"#, crate::work::json_str(&p_str)));
            } else {
                let _ = send_call("new_presentation", "{}");
            }
            let mut curr_title = String::new();
            let mut curr_body = Vec::new();
            let is_markdown = clean_content.lines().any(|l| l.trim().starts_with('#'));
            if is_markdown {
                for l in clean_content.lines() {
                    let t = l.trim();
                    if let Some(h) = t.strip_prefix('#') {
                        if !curr_title.is_empty() {
                            let _ = send_call(
                                "add_slide",
                                &format!(
                                    r#"{{"title":{},"body":{}}}"#,
                                    crate::work::json_str(&curr_title),
                                    crate::work::json_str(&curr_body.join("\n"))
                                ),
                            );
                            curr_body.clear();
                        }
                        curr_title = h.trim_start_matches('#').trim().to_string();
                    } else if !t.is_empty() {
                        let b = t.strip_prefix('-').or_else(|| t.strip_prefix('*')).unwrap_or(t).trim();
                        curr_body.push(b.to_string());
                    }
                }
                if !curr_title.is_empty() {
                    let _ = send_call(
                        "add_slide",
                        &format!(
                            r#"{{"title":{},"body":{}}}"#,
                            crate::work::json_str(&curr_title),
                            crate::work::json_str(&curr_body.join("\n"))
                        ),
                    );
                }
            } else {
                let _ = send_call(
                    "add_slide",
                    &format!(
                        r#"{{"title":"Presentation","body":{}}}"#,
                        crate::work::json_str(clean_content)
                    ),
                );
            }
            let save_res = send_call("save_presentation", &format!(r#"{{"path":{out_json_str}}}"#))?;
            if save_res.contains(r#""isError":true"#) {
                let _ = child.kill();
                return Err("DeckCraft save failed".into());
            }
        }
        "cadcraft" => {
            if let Some(target) = target_override {
                let p_str = target.to_string_lossy();
                let _ = send_call("open", &format!(r#"{{"path":{}}}"#, crate::work::json_str(&p_str)));
            } else {
                let _ = send_call("new_drawing", "{}");
            }
            let _ = send_call(
                "command_line",
                &format!(r#"{{"text":{}}}"#, crate::work::json_str(clean_content)),
            );
            let save_res = send_call("save", &format!(r#"{{"path":{out_json_str}}}"#))?;
            if save_res.contains(r#""isError":true"#) {
                let _ = child.kill();
                return Err("CADCraft save failed".into());
            }
        }
        _ => {
            let _ = child.kill();
            return Err("Unsupported slug".into());
        }
    }

    drop(stdin);
    let _ = child.wait();

    if out_path.is_file() {
        Ok(())
    } else {
        Err("MCP did not produce output file".into())
    }
}

#[cfg(not(windows))]
pub fn execute_craft_mcp_flow(
    _norm: &str,
    _cli: &Path,
    _out_path: &Path,
    _target_override: Option<&Path>,
    _clean_content: &str,
) -> Result<(), String> {
    Err("MCP flow only supported on Windows".into())
}

pub fn execute_craft_ai_block(
    slug: &str,
    content: &str,
    target_override: Option<&Path>,
) -> Result<CraftDocCard, String> {
    let lower = slug.to_ascii_lowercase();
    let norm = match lower.as_str() {
        "wordcraft" | "word" | "docx" => "wordcraft",
        "gridcraft" | "grid" | "xlsx" | "excel" | "csv" => "gridcraft",
        "deckcraft" | "deck" | "pptx" | "slides" | "presentation" => "deckcraft",
        "cadcraft" | "cad" | "dxf" => "cadcraft",
        other => other,
    };
    let (cli, _gui) = ensure_craft_installed(norm)?;
    let docs = vey_docs_dir();

    let mut target_filename = None;
    let mut clean_lines: Vec<String> = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            clean_lines.push(String::new());
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("--save") {
            target_filename = Some(rest.trim().trim_matches('"').trim().to_string());
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("save:") {
            target_filename = Some(rest.trim().trim_matches('"').trim().to_string());
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("filename:") {
            target_filename = Some(rest.trim().trim_matches('"').trim().to_string());
            continue;
        }
        clean_lines.push(line.to_string());
    }

    let clean_content = clean_lines.join("\n").trim().to_string();

    let def_ext = match norm {
        "deckcraft" => "deckcraft",
        "wordcraft" => "docx",
        "gridcraft" => "xlsx",
        "cadcraft" => "dxf",
        _ => "txt",
    };

    let (out_path, filename) = if let Some(target) = target_override {
        let fname = target
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("document")
            .to_string();
        (target.to_path_buf(), fname)
    } else {
        let fname = target_filename.unwrap_or_else(|| {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            format!("{norm}-{ts}.{def_ext}")
        });
        (docs.join(&fname), fname)
    };
    let temp_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let temp_base = std::env::temp_dir().join(format!("vey_{norm}_{temp_ts}"));

    #[cfg(windows)]
    {
        let mcp_res = execute_craft_mcp_flow(norm, &cli, &out_path, target_override, &clean_content);
        if mcp_res.is_err() || !out_path.is_file() {
            use std::os::windows::process::CommandExt;
            let mut cmd = std::process::Command::new(&cli);
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW

            match norm {
                "wordcraft" => {
                    let temp_md = temp_base.with_extension("md");
                    let _ = std::fs::write(&temp_md, &clean_content);
                    cmd.arg("convert").arg(&temp_md).arg(&out_path);
                }
                "gridcraft" => {
                    let temp_csv = temp_base.with_extension("csv");
                    let _ = std::fs::write(&temp_csv, &clean_content);
                    cmd.arg("convert").arg(&temp_csv).arg(&out_path);
                }
                "deckcraft" => {
                    let temp_cmds = temp_base.with_extension("txt");
                    let mut cmds = Vec::new();
                    let mut curr_title = String::new();
                    let mut curr_body = Vec::new();

                    let is_markdown = clean_content.lines().any(|l| l.trim().starts_with('#'));
                    if is_markdown {
                        for l in clean_content.lines() {
                            let t = l.trim();
                            if let Some(h) = t.strip_prefix('#') {
                                if !curr_title.is_empty() {
                                    let body_str = curr_body.join("\\n");
                                    cmds.push(format!(
                                        r#"slide.new={{"title":{},"body":{}}}"#,
                                        crate::work::json_str(&curr_title),
                                        crate::work::json_str(&body_str)
                                    ));
                                    curr_body.clear();
                                }
                                curr_title = h.trim_start_matches('#').trim().to_string();
                            } else if !t.is_empty() {
                                let b = t.strip_prefix('-').or_else(|| t.strip_prefix('*')).unwrap_or(t).trim();
                                curr_body.push(b.to_string());
                            }
                        }
                        if !curr_title.is_empty() {
                            let body_str = curr_body.join("\\n");
                            cmds.push(format!(
                                r#"slide.new={{"title":{},"body":{}}}"#,
                                crate::work::json_str(&curr_title),
                                crate::work::json_str(&body_str)
                            ));
                        }
                    } else {
                        for l in clean_content.lines() {
                            let t = l.trim();
                            if !t.is_empty() && !t.starts_with("//") {
                                cmds.push(t.to_string());
                            }
                        }
                    }

                    if cmds.is_empty() {
                        cmds.push(r#"slide.new={"title":"Presentation","body":""}"#.to_string());
                    }

                    let _ = std::fs::write(&temp_cmds, cmds.join("\n"));
                    cmd.arg("run").arg("--cmd-file").arg(&temp_cmds).arg("--save").arg(&out_path);
                }
                "cadcraft" => {
                    let temp_scr = temp_base.with_extension("scr");
                    let _ = std::fs::write(&temp_scr, &clean_content);
                    cmd.arg("run").arg("--script-file").arg(&temp_scr).arg("--save").arg(&out_path);
                }
                _ => {
                    return Err(format!("Unsupported craft slug: {slug}"));
                }
            }

            let st = cmd.status().map_err(|e| format!("Failed to run {}: {e}", cli.display()))?;
            if !st.success() && !out_path.is_file() {
                return Err(format!("{} exited with status {st}", cli.display()));
            }
        }
    }

    let size_bytes = std::fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
    let size_label = if size_bytes < 1024 {
        format!("{size_bytes} B")
    } else if size_bytes < 1024 * 1024 {
        format!("{:.1} KB", size_bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", size_bytes as f64 / (1024.0 * 1024.0))
    };

    let app_name = match slug.to_ascii_lowercase().as_str() {
        "deckcraft" => "DeckCraft",
        "wordcraft" => "WordCraft",
        "gridcraft" => "GridCraft",
        "cadcraft" => "CADCraft",
        _ => "ArtCraft",
    };

    Ok(CraftDocCard {
        slug: slug.to_string(),
        app_name: app_name.to_string(),
        file_name: filename,
        file_path: out_path,
        size_label,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    NotInstalled,
    Installing {
        step: String,
        pct: Option<u8>,
    },
    Installed {
        exe: String,
        version: Option<String>,
    },
    Failed(String),
}

impl Default for State {
    fn default() -> Self {
        State::NotInstalled
    }
}

#[derive(Clone, Default)]
pub struct CraftData {
    pub states: [State; CRAFT_COUNT],
    /// Consumed once by the UI thread: `(message, ok)`.
    pub toast: Option<(String, bool)>,
}

pub enum Req {
    Scan,
    Install(usize),
}

pub struct Handle {
    pub data: Arc<Mutex<CraftData>>,
    pub tx: Sender<Req>,
}

pub fn start(hwnd: isize) -> Handle {
    sync_vey_mcp_config();
    let initial = scan_all_states();
    let data = Arc::new(Mutex::new(CraftData {
        states: initial,
        toast: None,
    }));
    let (tx, rx) = channel();
    let shared = data.clone();
    std::thread::Builder::new()
        .name("craft".into())
        .spawn(move || worker(hwnd, shared, rx))
        .ok();
    Handle { data, tx }
}

pub fn scan_all_states() -> [State; CRAFT_COUNT] {
    let states = std::array::from_fn(|i| match find_installed(i) {
        Some((exe, version)) => State::Installed {
            exe: exe.to_string_lossy().into_owned(),
            version,
        },
        None => State::NotInstalled,
    });
    sync_vey_mcp_config();
    states
}

/// Managed portable directory for the i-th Craft app: `%APPDATA%\Vey\craft\<slug>`.
pub fn managed_dir(idx: usize) -> PathBuf {
    store::data_dir().join("craft").join(SPECS[idx].slug)
}

/// Locate an existing installation (Vey portable dir, Craft Launcher / MSI dirs, or PATH).
pub fn find_installed(idx: usize) -> Option<(PathBuf, Option<String>)> {
    let spec = &SPECS[idx];

    // 1. Vey-managed portable installation
    let mdir = managed_dir(idx);
    if let Some(exe) = find_exe_in_tree(&mdir, spec.exes, 3) {
        let ver = std::fs::read_to_string(mdir.join("version.txt"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        return Some((exe, ver));
    }

    // 2. Standard Windows installation folders (MSI / Craft Launcher)
    #[cfg(windows)]
    {
        let mut roots: Vec<PathBuf> = Vec::new();
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let l = PathBuf::from(local);
            roots.push(l.join("Programs"));
            roots.push(l.join("Programs").join("ArtCraft"));
            roots.push(l.join("ArtCraft"));
        }
        for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
            if let Some(pf) = std::env::var_os(var) {
                let p = PathBuf::from(pf);
                if !roots.contains(&p) {
                    roots.push(p.clone());
                }
                let ac = p.join("ArtCraft");
                if !roots.contains(&ac) {
                    roots.push(ac);
                }
            }
        }
        for root in &roots {
            for sub in spec.win_dirs {
                let dir = root.join(sub);
                for exe_name in spec.exes {
                    let candidate = dir.join(exe_name);
                    if candidate.is_file() {
                        return Some((candidate, None));
                    }
                }
            }
        }
    }

    // 3. System PATH
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            for exe_name in spec.exes {
                let candidate = dir.join(exe_name);
                if candidate.is_file() {
                    return Some((candidate, None));
                }
                #[cfg(not(windows))]
                {
                    let stem = exe_name.trim_end_matches(".exe");
                    let unix_cand = dir.join(stem);
                    if unix_cand.is_file() {
                        return Some((unix_cand, None));
                    }
                }
            }
        }
    }

    None
}

/// Recursively search `dir` (up to `max_depth`) for one of `exe_names` (exact filename match,
/// case-insensitive, so `wordcraft-cli.exe` is never confused with `wordcraft.exe`).
pub fn find_exe_in_tree(dir: &Path, exe_names: &[&str], max_depth: usize) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    for exe in exe_names {
        let direct = dir.join(exe);
        if direct.is_file() {
            return Some(direct);
        }
    }
    if max_depth == 0 {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Some(fname) = path.file_name().and_then(|s| s.to_str()) {
                if exe_names.iter().any(|want| fname.eq_ignore_ascii_case(want)) {
                    return Some(path);
                }
            }
        } else if path.is_dir() {
            subdirs.push(path);
        }
    }
    for sub in subdirs {
        if let Some(found) = find_exe_in_tree(&sub, exe_names, max_depth - 1) {
            return Some(found);
        }
    }
    None
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReleaseAsset {
    pub tag: String,
    pub name: String,
    pub url: String,
    pub size: u64,
    pub is_msi: bool,
}

pub fn current_windows_arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "windows-arm64"
    } else if cfg!(target_arch = "x86") {
        "windows-x86"
    } else {
        "windows-x64"
    }
}

/// Parse GitHub `/releases/latest` JSON and pick the best Windows asset for `arch`
/// (`"windows-x64"`, `"windows-arm64"`, or `"windows-x86"`). Prefers zero-admin portable `.zip`.
pub fn pick_windows_asset(json_body: &str, arch: &str) -> Option<ReleaseAsset> {
    let root = parse_json(json_body)?;
    let tag = root.str_or("tag_name");
    let assets = root.get("assets")?.as_arr()?;

    let mut parsed: Vec<(String, String, u64)> = Vec::new();
    for a in assets {
        let name = a.str_or("name");
        let url = a.str_or("browser_download_url");
        let size = match a.get("size") {
            Some(Json::Num(n)) if *n > 0.0 => *n as u64,
            _ => 0,
        };
        if !name.is_empty() && !url.is_empty() {
            parsed.push((name, url, size));
        }
    }

    let want_zip = format!("{arch}-portable.zip").to_ascii_lowercase();
    let x64_zip = "windows-x64-portable.zip";
    let want_msi = format!("{arch}.msi").to_ascii_lowercase();

    // 1. Exact architecture portable .zip
    if let Some((name, url, size)) = parsed
        .iter()
        .find(|(n, _, _)| n.to_ascii_lowercase().ends_with(&want_zip))
    {
        return Some(ReleaseAsset {
            tag,
            name: name.clone(),
            url: url.clone(),
            size: *size,
            is_msi: false,
        });
    }

    // 2. Fallback to windows-x64-portable.zip (e.g. x64 emulation on ARM64)
    if arch != "windows-x64" {
        if let Some((name, url, size)) = parsed
            .iter()
            .find(|(n, _, _)| n.to_ascii_lowercase().ends_with(x64_zip))
        {
            return Some(ReleaseAsset {
                tag,
                name: name.clone(),
                url: url.clone(),
                size: *size,
                is_msi: false,
            });
        }
    }

    // 3. Fallback to .msi installer
    if let Some((name, url, size)) = parsed
        .iter()
        .find(|(n, _, _)| n.to_ascii_lowercase().ends_with(&want_msi))
    {
        return Some(ReleaseAsset {
            tag,
            name: name.clone(),
            url: url.clone(),
            size: *size,
            is_msi: true,
        });
    }

    None
}

/// Split a full URL (`https://host:port/path?query`) preserving query parameters.
pub fn parse_full_url(raw: &str) -> Option<(bool, String, u16, String)> {
    let s = raw.trim();
    let (https, rest) = if let Some(r) = s.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = s.strip_prefix("http://") {
        (false, r)
    } else {
        return None;
    };
    let (authority, path_and_query) = match rest.find('/') {
        Some(idx) => (&rest[..idx], rest[idx..].to_string()),
        None => (rest, "/".to_string()),
    };
    if authority.is_empty() {
        return None;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() && !h.contains(']') => {
            let port = p.parse::<u16>().ok()?;
            (h.to_string(), port)
        }
        _ => (authority.to_string(), if https { 443 } else { 80 }),
    };
    Some((https, host, port, path_and_query))
}

#[cfg(windows)]
mod http {
    use std::ffi::c_void;
    use std::fs::File;
    use std::io::Write;
    use std::path::Path;
    use std::ptr::{null, null_mut};

    type HINTERNET = *mut c_void;

    const WINHTTP_ACCESS_TYPE_DEFAULT_PROXY: u32 = 0;
    const WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY: u32 = 4;
    const WINHTTP_FLAG_SECURE: u32 = 0x0080_0000;
    const WINHTTP_QUERY_STATUS_CODE: u32 = 19;
    const WINHTTP_QUERY_CONTENT_LENGTH: u32 = 5;
    const WINHTTP_QUERY_FLAG_NUMBER: u32 = 0x2000_0000;

    #[link(name = "winhttp")]
    extern "system" {
        fn WinHttpOpen(
            agent: *const u16,
            access_type: u32,
            proxy: *const u16,
            bypass: *const u16,
            flags: u32,
        ) -> HINTERNET;
        fn WinHttpSetTimeouts(
            h: HINTERNET,
            resolve_ms: i32,
            connect_ms: i32,
            send_ms: i32,
            recv_ms: i32,
        ) -> i32;
        fn WinHttpConnect(
            session: HINTERNET,
            server: *const u16,
            port: u16,
            reserved: u32,
        ) -> HINTERNET;
        fn WinHttpOpenRequest(
            connect: HINTERNET,
            verb: *const u16,
            object: *const u16,
            version: *const u16,
            referrer: *const u16,
            accept: *const *const u16,
            flags: u32,
        ) -> HINTERNET;
        fn WinHttpSendRequest(
            request: HINTERNET,
            headers: *const u16,
            headers_len: u32,
            optional: *const c_void,
            optional_len: u32,
            total_len: u32,
            context: usize,
        ) -> i32;
        fn WinHttpReceiveResponse(request: HINTERNET, reserved: *mut c_void) -> i32;
        fn WinHttpQueryHeaders(
            request: HINTERNET,
            info_level: u32,
            name: *const u16,
            buffer: *mut c_void,
            buffer_len: *mut u32,
            index: *mut u32,
        ) -> i32;
        fn WinHttpReadData(
            request: HINTERNET,
            buffer: *mut c_void,
            to_read: u32,
            read: *mut u32,
        ) -> i32;
        fn WinHttpCloseHandle(h: HINTERNET) -> i32;
    }

    struct HandleGuard(HINTERNET);
    impl Drop for HandleGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    WinHttpCloseHandle(self.0);
                }
            }
        }
    }

    pub struct Client {
        session: HINTERNET,
    }

    impl Client {
        pub fn new() -> Self {
            let agent = wide("Vey/0.13");
            let mut s = unsafe {
                WinHttpOpen(
                    agent.as_ptr(),
                    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                    null(),
                    null(),
                    0,
                )
            };
            if s.is_null() {
                s = unsafe {
                    WinHttpOpen(
                        agent.as_ptr(),
                        WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
                        null(),
                        null(),
                        0,
                    )
                };
            }
            if !s.is_null() {
                unsafe {
                    WinHttpSetTimeouts(s, 8_000, 12_000, 15_000, 60_000);
                }
            }
            Client { session: s }
        }

        pub fn get_text(&self, url: &str) -> Result<(u32, String), String> {
            if self.session.is_null() {
                return Err("WinHTTP unavailable".into());
            }
            let (https, host, port, path) =
                super::parse_full_url(url).ok_or_else(|| "Invalid URL".to_string())?;
            let host_w = wide(&host);
            let conn = HandleGuard(unsafe { WinHttpConnect(self.session, host_w.as_ptr(), port, 0) });
            if conn.0.is_null() {
                return Err(format!("Cannot connect to {host}"));
            }
            let verb_w = wide("GET");
            let path_w = wide(&path);
            let flags = if https { WINHTTP_FLAG_SECURE } else { 0 };
            let req = HandleGuard(unsafe {
                WinHttpOpenRequest(conn.0, verb_w.as_ptr(), path_w.as_ptr(), null(), null(), null(), flags)
            });
            if req.0.is_null() {
                return Err("Cannot open request".into());
            }
            let hdr_w = wide("Accept: application/vnd.github+json\r\n");
            let sent = unsafe {
                WinHttpSendRequest(req.0, hdr_w.as_ptr(), u32::MAX, null(), 0, 0, 0)
            };
            if sent == 0 {
                return Err(format!("Cannot reach {host}"));
            }
            if unsafe { WinHttpReceiveResponse(req.0, null_mut()) } == 0 {
                return Err(format!("No response from {host}"));
            }
            let status = query_u32(req.0, WINHTTP_QUERY_STATUS_CODE);
            let mut out = Vec::new();
            let mut buf = [0u8; 8192];
            loop {
                let mut n: u32 = 0;
                let ok = unsafe {
                    WinHttpReadData(req.0, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut n)
                };
                if ok == 0 || n == 0 {
                    break;
                }
                out.extend_from_slice(&buf[..n as usize]);
            }
            Ok((status, String::from_utf8_lossy(&out).into_owned()))
        }

        pub fn download_to_file(
            &self,
            url: &str,
            dest: &Path,
            expected_size: u64,
            mut on_progress: impl FnMut(u8),
        ) -> Result<(), String> {
            if self.session.is_null() {
                return Err("WinHTTP unavailable".into());
            }
            let (https, host, port, path) =
                super::parse_full_url(url).ok_or_else(|| "Invalid download URL".to_string())?;
            let host_w = wide(&host);
            let conn = HandleGuard(unsafe { WinHttpConnect(self.session, host_w.as_ptr(), port, 0) });
            if conn.0.is_null() {
                return Err(format!("Cannot connect to {host}"));
            }
            let verb_w = wide("GET");
            let path_w = wide(&path);
            let flags = if https { WINHTTP_FLAG_SECURE } else { 0 };
            let req = HandleGuard(unsafe {
                WinHttpOpenRequest(conn.0, verb_w.as_ptr(), path_w.as_ptr(), null(), null(), null(), flags)
            });
            if req.0.is_null() {
                return Err("Cannot open download request".into());
            }
            let hdr_w = wide("Accept: application/octet-stream\r\n");
            let sent = unsafe {
                WinHttpSendRequest(req.0, hdr_w.as_ptr(), u32::MAX, null(), 0, 0, 0)
            };
            if sent == 0 {
                return Err("Download connection failed".into());
            }
            if unsafe { WinHttpReceiveResponse(req.0, null_mut()) } == 0 {
                return Err("Download timed out".into());
            }
            let status = query_u32(req.0, WINHTTP_QUERY_STATUS_CODE);
            if !(200..=299).contains(&status) {
                return Err(format!("HTTP {status}"));
            }
            let hdr_len = query_u32(req.0, WINHTTP_QUERY_CONTENT_LENGTH) as u64;
            let total = if expected_size > 0 { expected_size } else { hdr_len };

            let mut file = File::create(dest).map_err(|_| "Cannot create temp file".to_string())?;
            let mut buf = vec![0u8; 65_536];
            let mut received: u64 = 0;
            let mut last_pct: u8 = 0;
            loop {
                let mut n: u32 = 0;
                let ok = unsafe {
                    WinHttpReadData(req.0, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut n)
                };
                if ok == 0 {
                    return Err("Connection interrupted".into());
                }
                if n == 0 {
                    break;
                }
                file.write_all(&buf[..n as usize])
                    .map_err(|_| "Disk write error".to_string())?;
                received += n as u64;
                if total > 0 {
                    let pct = ((received.saturating_mul(100)) / total).min(100) as u8;
                    if pct >= last_pct.saturating_add(2) || pct == 100 {
                        last_pct = pct;
                        on_progress(pct);
                    }
                }
            }
            file.flush().ok();
            if received < 100_000 {
                return Err("Incomplete download".into());
            }
            Ok(())
        }
    }

    fn query_u32(req: HINTERNET, flag: u32) -> u32 {
        let mut val: u32 = 0;
        let mut sz = std::mem::size_of::<u32>() as u32;
        unsafe {
            WinHttpQueryHeaders(
                req,
                flag | WINHTTP_QUERY_FLAG_NUMBER,
                null(),
                &mut val as *mut u32 as *mut c_void,
                &mut sz,
                null_mut(),
            );
        }
        val
    }

    impl Drop for Client {
        fn drop(&mut self) {
            if !self.session.is_null() {
                unsafe {
                    WinHttpCloseHandle(self.session);
                }
            }
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(windows)]
fn post_ui(hwnd: isize) {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
    if hwnd != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_CRAFT, WPARAM(0), LPARAM(0));
        }
    }
}

#[cfg(windows)]
fn worker(hwnd: isize, shared: Arc<Mutex<CraftData>>, rx: Receiver<Req>) {
    let client = http::Client::new();
    while let Ok(req) = rx.recv() {
        match req {
            Req::Scan => {
                let scanned = scan_all_states();
                let mut d = shared.lock().unwrap();
                for (i, st) in scanned.into_iter().enumerate() {
                    if !matches!(d.states[i], State::Installing { .. }) {
                        d.states[i] = st;
                    }
                }
                drop(d);
                post_ui(hwnd);
            }
            Req::Install(idx) => {
                if idx < CRAFT_COUNT {
                    do_install(&client, hwnd, &shared, idx);
                }
            }
        }
    }
}

#[cfg(windows)]
fn set_state(hwnd: isize, shared: &Arc<Mutex<CraftData>>, idx: usize, st: State, toast: Option<(String, bool)>) {
    let mut d = shared.lock().unwrap();
    d.states[idx] = st;
    if toast.is_some() {
        d.toast = toast;
    }
    drop(d);
    post_ui(hwnd);
}

#[cfg(windows)]
fn extract_zip(zip_path: &Path, dest_dir: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let _ = std::fs::remove_dir_all(dest_dir);
    std::fs::create_dir_all(dest_dir).map_err(|_| "Cannot create install folder".to_string())?;

    // Fast path: Windows built-in bsdtar (C:\Windows\System32\tar.exe)
    let tar_ok = Command::new("tar.exe")
        .args(["-xf", &zip_path.to_string_lossy(), "-C", &dest_dir.to_string_lossy()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .is_ok_and(|s| s.success());
    if tar_ok {
        return Ok(());
    }

    // Fallback: PowerShell Expand-Archive
    let zp = zip_path.to_string_lossy().replace('\'', "''");
    let dp = dest_dir.to_string_lossy().replace('\'', "''");
    let ps = format!(
        "$ProgressPreference='SilentlyContinue'; Expand-Archive -LiteralPath '{zp}' -DestinationPath '{dp}' -Force"
    );
    let ps_ok = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &ps])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .is_ok_and(|s| s.success());
    if ps_ok {
        Ok(())
    } else {
        Err("Archive extraction failed".into())
    }
}

#[cfg(windows)]
fn do_install(client: &http::Client, hwnd: isize, shared: &Arc<Mutex<CraftData>>, idx: usize) {
    let spec = &SPECS[idx];
    set_state(
        hwnd,
        shared,
        idx,
        State::Installing {
            step: "Checking release\u{2026}".into(),
            pct: None,
        },
        None,
    );

    let api_url = format!("https://api.github.com/repos/{}/releases/latest", spec.repo);
    let body = match client.get_text(&api_url) {
        Ok((200..=299, b)) => b,
        Ok((code, _)) => {
            set_state(
                hwnd,
                shared,
                idx,
                State::Failed(format!("GitHub API HTTP {code}")),
                Some((format!("{} download failed", spec.name), false)),
            );
            return;
        }
        Err(e) => {
            set_state(
                hwnd,
                shared,
                idx,
                State::Failed(e),
                Some((format!("{} download failed", spec.name), false)),
            );
            return;
        }
    };

    let Some(asset) = pick_windows_asset(&body, current_windows_arch()) else {
        set_state(
            hwnd,
            shared,
            idx,
            State::Failed("No Windows package in latest release".into()),
            Some((format!("No package found for {}", spec.name), false)),
        );
        return;
    };

    let step_label = format!("Downloading {}", asset.tag);
    set_state(
        hwnd,
        shared,
        idx,
        State::Installing {
            step: step_label.clone(),
            pct: Some(0),
        },
        None,
    );

    let ext = if asset.is_msi { "msi" } else { "zip" };
    let tmp_pkg = std::env::temp_dir().join(format!("vey-{}-{}.{}", spec.slug, std::process::id(), ext));
    let _ = std::fs::remove_file(&tmp_pkg);

    let dl = client.download_to_file(&asset.url, &tmp_pkg, asset.size, |pct| {
        set_state(
            hwnd,
            shared,
            idx,
            State::Installing {
                step: step_label.clone(),
                pct: Some(pct),
            },
            None,
        );
    });

    if let Err(e) = dl {
        let _ = std::fs::remove_file(&tmp_pkg);
        set_state(
            hwnd,
            shared,
            idx,
            State::Failed(e),
            Some((format!("{} download failed", spec.name), false)),
        );
        return;
    }

    if asset.is_msi {
        set_state(
            hwnd,
            shared,
            idx,
            State::Installing {
                step: "Running installer\u{2026}".into(),
                pct: Some(100),
            },
            None,
        );
        let _ = std::process::Command::new("msiexec.exe")
            .args(["/i", &tmp_pkg.to_string_lossy(), "/passive"])
            .status();
        let _ = std::fs::remove_file(&tmp_pkg);
        if let Some((exe, _)) = find_installed(idx) {
            set_state(
                hwnd,
                shared,
                idx,
                State::Installed {
                    exe: exe.to_string_lossy().into_owned(),
                    version: Some(asset.tag),
                },
                Some((format!("{} installed", spec.name), true)),
            );
        } else {
            set_state(hwnd, shared, idx, State::NotInstalled, None);
        }
        return;
    }

    set_state(
        hwnd,
        shared,
        idx,
        State::Installing {
            step: "Extracting\u{2026}".into(),
            pct: Some(100),
        },
        None,
    );

    let craft_root = store::data_dir().join("craft");
    let _ = std::fs::create_dir_all(&craft_root);
    let stage_dir = craft_root.join(format!("{}.tmp", spec.slug));
    let final_dir = managed_dir(idx);

    if let Err(e) = extract_zip(&tmp_pkg, &stage_dir) {
        let _ = std::fs::remove_file(&tmp_pkg);
        let _ = std::fs::remove_dir_all(&stage_dir);
        set_state(
            hwnd,
            shared,
            idx,
            State::Failed(e),
            Some((format!("{} extraction failed", spec.name), false)),
        );
        return;
    }
    let _ = std::fs::remove_file(&tmp_pkg);

    if find_exe_in_tree(&stage_dir, spec.exes, 3).is_none() {
        let _ = std::fs::remove_dir_all(&stage_dir);
        set_state(
            hwnd,
            shared,
            idx,
            State::Failed("Executable missing in archive".into()),
            Some((format!("{} archive invalid", spec.name), false)),
        );
        return;
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        for exe in spec.exes {
            let _ = std::process::Command::new("taskkill")
                .args(["/F", "/IM", exe])
                .creation_flags(0x08000000)
                .status();
        }
    }

    let mut renamed = false;
    for _ in 0..4 {
        let _ = std::fs::remove_dir_all(&final_dir);
        if std::fs::rename(&stage_dir, &final_dir).is_ok() {
            renamed = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    if !renamed {
        set_state(
            hwnd,
            shared,
            idx,
            State::Failed("Could not finalize folder (app running?)".into()),
            Some((format!("Close {} and retry", spec.name), false)),
        );
        return;
    }
    let _ = std::fs::write(final_dir.join("version.txt"), &asset.tag);

    match find_exe_in_tree(&final_dir, spec.exes, 3) {
        Some(exe) => {
            set_state(
                hwnd,
                shared,
                idx,
                State::Installed {
                    exe: exe.to_string_lossy().into_owned(),
                    version: Some(asset.tag.clone()),
                },
                Some((format!("{} {} ready to launch", spec.name, asset.tag), true)),
            );
        }
        None => {
            set_state(
                hwnd,
                shared,
                idx,
                State::Failed("Installed binary not found".into()),
                None,
            );
        }
    }
}

#[cfg(not(windows))]
fn worker(_hwnd: isize, shared: Arc<Mutex<CraftData>>, rx: Receiver<Req>) {
    while let Ok(req) = rx.recv() {
        if let Req::Scan = req {
            let scanned = scan_all_states();
            let mut d = shared.lock().unwrap();
            d.states = scanned;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_portable_zip_for_each_craft_app_and_arch() {
        let launcher_json = r#"{
            "tag_name": "v0.16.0",
            "assets": [
                {"name": "artcraft-launcher-0.16.0-windows-arm64.msi", "browser_download_url": "https://example.com/arm64.msi", "size": 9300000},
                {"name": "artcraft-launcher-0.16.0-windows-x64.msi", "browser_download_url": "https://example.com/x64.msi", "size": 10000000},
                {"name": "artcraft-launcher-0.16.0-windows-x64-portable.zip", "browser_download_url": "https://example.com/x64-portable.zip", "size": 11200000},
                {"name": "artcraft-launcher-0.16.0-windows-arm64-portable.zip", "browser_download_url": "https://example.com/arm64-portable.zip", "size": 10500000}
            ]
        }"#;
        let x64 = pick_windows_asset(launcher_json, "windows-x64").unwrap();
        assert_eq!(x64.tag, "v0.16.0");
        assert_eq!(x64.name, "artcraft-launcher-0.16.0-windows-x64-portable.zip");
        assert!(!x64.is_msi);
        assert_eq!(x64.size, 11_200_000);

        let arm64 = pick_windows_asset(launcher_json, "windows-arm64").unwrap();
        assert_eq!(arm64.name, "artcraft-launcher-0.16.0-windows-arm64-portable.zip");
        assert!(!arm64.is_msi);

        // x86 falls back to x64 portable zip when x86 asset is absent
        let x86_fallback = pick_windows_asset(launcher_json, "windows-x86").unwrap();
        assert_eq!(x86_fallback.name, "artcraft-launcher-0.16.0-windows-x64-portable.zip");

        // MSI fallback when portable zip is absent
        let msi_only = r#"{
            "tag_name": "v0.18.0",
            "assets": [
                {"name": "gridcraft-0.18.0-windows-x64.msi", "browser_download_url": "https://example.com/gc.msi", "size": 40000000}
            ]
        }"#;
        let msi = pick_windows_asset(msi_only, "windows-x64").unwrap();
        assert!(msi.is_msi);
        assert_eq!(msi.name, "gridcraft-0.18.0-windows-x64.msi");
    }

    #[test]
    fn preserves_query_string_in_download_urls() {
        let u = "https://release-assets.githubusercontent.com/github-production-release-asset/123?sp=r&sv=2018-11-09&sig=abc%2Bdef";
        let (https, host, port, pq) = parse_full_url(u).unwrap();
        assert!(https);
        assert_eq!(host, "release-assets.githubusercontent.com");
        assert_eq!(port, 443);
        assert_eq!(
            pq,
            "/github-production-release-asset/123?sp=r&sv=2018-11-09&sig=abc%2Bdef"
        );
    }

    #[test]
    fn tree_lookup_ignores_cli_binaries() {
        let tmp = std::env::temp_dir().join(format!("vey-craft-test-{}", std::process::id()));
        let nested = tmp.join("wordcraft-0.16.0-windows-x64");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("wordcraft-cli.exe"), b"cli").unwrap();
        assert!(find_exe_in_tree(&tmp, &["wordcraft.exe"], 3).is_none());

        std::fs::write(nested.join("wordcraft.exe"), b"gui").unwrap();
        let found = find_exe_in_tree(&tmp, &["wordcraft.exe"], 3).unwrap();
        assert!(found.ends_with("wordcraft.exe"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn executes_wordcraft_markdown_to_docx() {
        if find_craft_cli("wordcraft").is_none() {
            return;
        }
        let block = "save: test_ai_essay.docx\n# AI in Modern Engineering\nParagraph content.";
        let res = execute_craft_ai_block("wordcraft", block, None).expect("wordcraft execution");
        assert_eq!(res.file_name, "test_ai_essay.docx");
        assert!(res.file_path.is_file());

        // Test editing existing file in-place with target_override
        let edit_block = "# AI in Modern Engineering (Revised)\nUpdated paragraph.";
        let res_edit = execute_craft_ai_block("wordcraft", edit_block, Some(&res.file_path))
            .expect("wordcraft edit execution");
        assert_eq!(res_edit.file_path, res.file_path);
    }

    #[test]
    fn executes_gridcraft_csv_to_xlsx() {
        if find_craft_cli("gridcraft").is_none() {
            return;
        }
        let block = "save: test_ai_sheet.xlsx\nQuarter,Revenue,Expenses\nQ1,100,50";
        let res = execute_craft_ai_block("gridcraft", block, None).expect("gridcraft execution");
        assert_eq!(res.file_name, "test_ai_sheet.xlsx");
        assert!(res.file_path.is_file());
    }

    #[test]
    fn executes_deckcraft_outline_to_deckcraft() {
        if find_craft_cli("deckcraft").is_none() {
            return;
        }
        let block = "save: test_ai_deck.deckcraft\n# Slide One\n- Point A\n# Slide Two\n- Point B";
        let res = execute_craft_ai_block("deckcraft", block, None).expect("deckcraft execution");
        assert_eq!(res.file_name, "test_ai_deck.deckcraft");
        assert!(res.file_path.is_file());
    }

    #[test]
    fn executes_wordcraft_mcp_flow_directly() {
        if let Some(cli) = find_craft_cli("wordcraft") {
            let out = vey_docs_dir().join("test_mcp_direct.docx");
            let _ = std::fs::remove_file(&out);
            let res = execute_craft_mcp_flow(
                "wordcraft",
                &cli,
                &out,
                None,
                "Testing direct MCP session in WordCraft",
            );
            assert!(res.is_ok());
            assert!(out.is_file());
            let _ = std::fs::remove_file(&out);
        }
    }

    #[test]
    fn executes_gridcraft_mcp_flow_directly() {
        if let Some(cli) = find_craft_cli("gridcraft") {
            let out = vey_docs_dir().join("test_mcp_direct.xlsx");
            let _ = std::fs::remove_file(&out);
            let res = execute_craft_mcp_flow(
                "gridcraft",
                &cli,
                &out,
                None,
                "ColA,ColB\n10,20",
            );
            assert!(res.is_ok());
            assert!(out.is_file());
            let _ = std::fs::remove_file(&out);
        }
    }

    #[test]
    fn executes_deckcraft_mcp_flow_directly() {
        if let Some(cli) = find_craft_cli("deckcraft") {
            let out = vey_docs_dir().join("test_mcp_direct.deckcraft");
            let _ = std::fs::remove_file(&out);
            let res = execute_craft_mcp_flow(
                "deckcraft",
                &cli,
                &out,
                None,
                "# MCP Slide\n- Bullet item",
            );
            assert!(res.is_ok());
            assert!(out.is_file());
            let _ = std::fs::remove_file(&out);
        }
    }

    #[test]
    fn syncs_vey_mcp_config_dynamically() {
        let path = sync_vey_mcp_config();
        assert!(path.is_some());
        let p = path.unwrap();
        assert!(p.is_file());
        let content = std::fs::read_to_string(&p).expect("read mcp.json");
        assert!(content.contains("\"mcpServers\""));
    }

    #[test]
    fn fetches_mcp_tools_catalog() {
        if find_craft_cli("wordcraft").is_some() {
            let tools = fetch_craft_mcp_tools("wordcraft", None).expect("fetch tools");
            assert!(!tools.is_empty());
            assert!(tools.iter().any(|t| t.name == "new_document"));
            assert!(tools.iter().any(|t| t.name == "type_text"));
            assert!(tools.iter().any(|t| t.name == "save_document"));
        }
    }
}
