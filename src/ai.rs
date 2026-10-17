//! Optional AI helpers: explain a log or error, summarize, rewrite the
//! selection, and tidy messy notes into Markdown.
//!
//! Off by default. Two providers:
//! * "local": an OpenAI-compatible server on this PC (Ollama, LM Studio,
//!   llama.cpp). Text never leaves the machine.
//! * "anthropic": the Claude API (Messages API over HTTPS). The API key is
//!   encrypted with Windows DPAPI for the current user, and Nib asks before
//!   the first request sends text off the machine.
//!
//! Requests run on a worker thread; results come back as WM_APP_AI.

use crate::app::{self, app, set_status, Info, InfoAction, WM_APP_AI};
use crate::gfx::icon;
use crate::palette::{self, Item, Kind};
use crate::{compare, docs, http, languages, util};
use serde_json::{json, Value};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";

#[derive(Clone, Debug)]
pub enum Task {
    Explain,
    Summarize,
    Rewrite(String),
    Format,
}

impl Task {
    fn label(&self) -> &'static str {
        match self {
            Task::Explain => "Explanation",
            Task::Summarize => "Summary",
            Task::Rewrite(_) => "Rewrite",
            Task::Format => "Formatted notes",
        }
    }

    fn system(&self) -> String {
        match self {
            Task::Explain => "You help a developer understand text they pasted into their editor: usually log output, an error message, a stack trace or code. Explain what happened, the most likely cause, and concrete next steps. Quote the key lines you rely on. Answer in concise Markdown with short sections. If the text is not technical, explain its meaning plainly.".into(),
            Task::Summarize => "Summarize the user's text in Markdown: a one-sentence overview, then the key points as bullets, then any decisions, dates or action items as a checklist. Keep the original language of the text. Do not add information that isn't in the text.".into(),
            Task::Rewrite(instr) => format!("Rewrite the user's text. Instruction: {instr}. Keep the meaning, the language and any Markdown or code formatting. Reply with only the rewritten text: no preamble, no quotes, no explanation."),
            Task::Format => "Turn the user's messy notes into clean, well-organized Markdown: a short title if one is obvious, headings for topics, bullet lists, '- [ ]' checkboxes for tasks, and consistent spacing. Keep every piece of information and the original language; do not invent content or add commentary. Reply with only the Markdown.".into(),
        }
    }

    /// Whether the result replaces the input (rather than opening in a new tab).
    fn replaces(&self) -> bool {
        matches!(self, Task::Rewrite(_) | Task::Format)
    }
}

struct Job {
    task: Task,
    doc: u64,
    range: (usize, usize),
    original: Vec<u8>,
    result: Result<String, String>,
}

// ------------------------------------------------------------------ settings and keys

pub fn enable(provider: &'static str) {
    let mut s = app::settings().clone();
    s.ai.enabled = true;
    s.ai.provider = provider.to_string();
    app::apply_settings(s);
    docs::clear_info();
    if provider == "anthropic" && api_key().is_none() {
        prompt_key();
    } else {
        set_status(if provider == "local" { "AI uses a local model. Run Ollama (or another OpenAI-compatible server) and pick the model in settings.json." } else { "AI uses the Claude API" });
    }
}

pub fn pick_provider() {
    let items = vec![
        Item::new(icon::CLOSE, "Off", Kind::Index(0)).detail("no AI features"),
        Item::new(icon::SPARKLE, "Local model (Ollama, LM Studio…)", Kind::Index(1)).detail("text stays on this PC"),
        Item::new(icon::SPARKLE, "Claude API (Anthropic)", Kind::Index(2)).detail("sends the text you choose to api.anthropic.com"),
        Item::new(icon::SETTINGS, "Edit AI settings…", Kind::Index(3)).detail("endpoint, model, effort, input limit"),
    ];
    palette::pick("AI provider", items, true, |i| match i {
        0 => {
            let mut s = app::settings().clone();
            s.ai.enabled = false;
            app::apply_settings(s);
            set_status("AI features are off");
        }
        1 => enable("local"),
        2 => enable("anthropic"),
        _ => crate::commands::open_settings_file(),
    });
}

pub fn prompt_key() {
    palette::prompt("Paste your Claude API key (stored encrypted for your Windows account)", "", true, |key| {
        let key = key.trim().to_string();
        if key.is_empty() {
            return;
        }
        match dpapi::protect(key.as_bytes()) {
            Some(blob) => {
                let mut s = app::settings().clone();
                s.ai.anthropic_key_protected = base64::encode(&blob);
                app::apply_settings(s);
                set_status("API key saved");
            }
            None => set_status("Couldn't encrypt the key"),
        }
    });
}

fn api_key() -> Option<String> {
    let enc = app::settings().ai.anthropic_key_protected.clone();
    if enc.is_empty() {
        return std::env::var("ANTHROPIC_API_KEY").ok().filter(|k| !k.is_empty());
    }
    let blob = base64::decode(&enc)?;
    dpapi::unprotect(&blob).and_then(|b| String::from_utf8(b).ok())
}

// ------------------------------------------------------------------ running

pub fn pick_rewrite() {
    let tones = ["Make it clearer", "Make it shorter", "Make it more formal", "Make it friendlier", "Fix spelling and grammar", "Turn into bullet points", "Custom instruction…"];
    let items = tones.iter().enumerate().map(|(i, t)| Item::new(icon::SPARKLE, *t, Kind::Index(i))).collect();
    palette::pick("Rewrite the selection with AI", items, true, move |i| {
        if i == tones.len() - 1 {
            palette::prompt("How should the text be rewritten?", "", false, |instr| {
                if !instr.trim().is_empty() {
                    run_task(Task::Rewrite(instr));
                }
            });
        } else {
            run_task(Task::Rewrite(tones[i].to_string()));
        }
    });
}

fn offer_enable() {
    docs::set_info(Info {
        icon: icon::SPARKLE,
        text: "AI features are optional and off. Choose where text is sent:".into(),
        actions: vec![("Local model (Ollama)".into(), InfoAction::EnableAi("local")), ("Claude API".into(), InfoAction::EnableAi("anthropic")), ("Not now".into(), InfoAction::Dismiss)],
        doc: 0,
        warning: false,
    });
}

pub fn run_task(task: Task) {
    let s = app::settings().ai.clone();
    if !s.enabled {
        offer_enable();
        return;
    }
    let g = app::active_group();
    let v = app::view(g);
    let doc = app::active_doc();
    let (a, b) = if v.sel_empty() {
        if matches!(task, Task::Rewrite(_)) {
            set_status("Select the text to rewrite first");
            return;
        }
        (0, v.len())
    } else {
        (v.sel_start(), v.sel_end())
    };
    let text = v.range(a, b);
    if text.iter().all(|c| c.is_ascii_whitespace()) {
        set_status("There's no text to send");
        return;
    }
    let limit = s.max_input_kb as usize * 1024;
    if text.len() > limit {
        util::message_box(
            app().hwnd.get(),
            &format!("The text is {}, over the AI input limit of {}.\n\nSelect a smaller part, or raise \"ai.max_input_kb\" in settings.", util::human_bytes(text.len() as u64), util::human_bytes(limit as u64)),
            MB_OK | MB_ICONINFORMATION,
        );
        return;
    }
    if s.provider == "anthropic" {
        if api_key().is_none() {
            prompt_key();
            return;
        }
        if !app().st.borrow().persist.ai_remote_ok {
            let r = util::message_box(
                app().hwnd.get(),
                &format!("Send {} of text to Anthropic's Claude API (api.anthropic.com)?\n\nNib only sends the selection (or this document) when you run an AI command. You won't be asked again.", util::human_bytes(text.len() as u64)),
                MB_OKCANCEL | MB_ICONQUESTION,
            );
            if r != IDOK {
                return;
            }
            app().st.borrow_mut().persist.ai_remote_ok = true;
            app::save_persist();
        }
    }
    let key = api_key();
    let hwnd = app().hwnd.get() as usize;
    set_status(&format!("AI: working on {}…", task.label().to_lowercase()));
    let input = String::from_utf8_lossy(&text).into_owned();
    let t2 = task.clone();
    std::thread::spawn(move || {
        let result = match s.provider.as_str() {
            "anthropic" => call_anthropic(&s, key.as_deref().unwrap_or(""), &t2.system(), &input),
            _ => call_local(&s, &t2.system(), &input),
        };
        let job = Box::new(Job { task: t2, doc, range: (a, b), original: text, result });
        unsafe { PostMessageW(hwnd as _, WM_APP_AI, 0, Box::into_raw(job) as isize) };
    });
}

fn call_anthropic(s: &crate::settings::AiSettings, key: &str, system: &str, input: &str) -> Result<String, String> {
    let body = json!({
        "model": s.anthropic_model,
        "max_tokens": 16000,
        "system": system,
        "messages": [{ "role": "user", "content": input }],
        "output_config": { "effort": s.anthropic_effort },
        "fallbacks": "default",
    });
    let headers = [
        ("content-type", "application/json".to_string()),
        ("x-api-key", key.to_string()),
        ("anthropic-version", "2023-06-01".to_string()),
        ("anthropic-beta", "server-side-fallback-2026-07-01".to_string()),
    ];
    let (status, resp) = http::post(ANTHROPIC_URL, &headers, body.to_string().as_bytes(), 300_000)?;
    let v: Value = serde_json::from_slice(&resp).map_err(|_| format!("Unexpected response (HTTP {status})"))?;
    if status != 200 {
        let msg = v["error"]["message"].as_str().unwrap_or("request failed");
        return Err(format!("Claude API error (HTTP {status}): {msg}"));
    }
    if v["stop_reason"] == "refusal" {
        let why = v["stop_details"]["explanation"].as_str().unwrap_or("");
        return Err(format!("The model declined this request. {why}").trim().to_string());
    }
    let text: String = v["content"].as_array().map(|blocks| blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("")).unwrap_or_default();
    if text.trim().is_empty() {
        return Err("The model returned no text".into());
    }
    let mut out = text;
    if v["stop_reason"] == "max_tokens" {
        out.push_str("\n\n[Output was cut off at the length limit.]");
    }
    Ok(out)
}

fn call_local(s: &crate::settings::AiSettings, system: &str, input: &str) -> Result<String, String> {
    let url = format!("{}/chat/completions", s.local_endpoint.trim_end_matches('/'));
    let body = json!({
        "model": s.local_model,
        "messages": [{ "role": "system", "content": system }, { "role": "user", "content": input }],
        "stream": false,
    });
    let headers = [("content-type", "application/json".to_string())];
    let (status, resp) = http::post(&url, &headers, body.to_string().as_bytes(), 300_000).map_err(|e| format!("{e}\n\nIs your local model server running at {}?", s.local_endpoint))?;
    let v: Value = serde_json::from_slice(&resp).map_err(|_| format!("Unexpected response from {url} (HTTP {status})"))?;
    if status != 200 {
        let msg = v["error"]["message"].as_str().or(v["error"].as_str()).unwrap_or("request failed");
        return Err(format!("Local model error (HTTP {status}): {msg}"));
    }
    v["choices"][0]["message"]["content"].as_str().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).ok_or_else(|| "The model returned no text".into())
}

pub fn on_result(lp: isize) {
    let job = unsafe { Box::from_raw(lp as *mut Job) };
    let Job { task, doc, range, original, result } = *job;
    let text = match result {
        Ok(t) => t,
        Err(e) => {
            set_status("AI request failed");
            util::error_box(app().hwnd.get(), &e);
            return;
        }
    };
    set_status(&format!("AI: {} ready", task.label().to_lowercase()));
    // Replace in place only if the original text is still exactly where it was.
    let shown_in = app().st.borrow().shown_in(doc).first().copied();
    if task.replaces() {
        if let Some(g) = shown_in {
            let v = app::view(g);
            if v.range(range.0, range.1) == original && !v.readonly() {
                let eol = match v.m(crate::sci_consts::SCI_GETEOLMODE, 0, 0) {
                    crate::sci_consts::SC_EOL_CRLF => "\r\n",
                    crate::sci_consts::SC_EOL_CR => "\r",
                    _ => "\n",
                };
                let new = text.replace("\r\n", "\n").replace('\n', eol);
                v.begin_undo();
                v.replace_range(range.0, range.1, new.as_bytes());
                v.end_undo();
                v.set_sel(range.0, range.0 + new.len());
                docs::set_info(Info {
                    icon: icon::SPARKLE,
                    text: format!("{} applied. Review it before saving.", task.label()),
                    actions: vec![("Compare with before".into(), InfoAction::AiCompareBefore), ("Undo".into(), InfoAction::Undo), ("OK".into(), InfoAction::Dismiss)],
                    doc,
                    warning: false,
                });
                BEFORE.with(|b| *b.borrow_mut() = Some((doc, original, range.0, new.len())));
                return;
            }
        }
    }
    let title = app().st.borrow().doc(doc).map(|d| d.name()).unwrap_or_default();
    let body = if task.replaces() { text } else { format!("# {} of {title}\n\n{text}\n", task.label()) };
    docs::new_doc_with(body.replace("\r\n", "\n").replace('\n', "\r\n").into_bytes(), Some(languages::by_id("markdown")));
}

thread_local! {
    /// (doc, original text, start, length of the AI text) for "Compare with before".
    static BEFORE: std::cell::RefCell<Option<(u64, Vec<u8>, usize, usize)>> = const { std::cell::RefCell::new(None) };
}

/// Compares the document with its text from before the last AI edit.
pub fn compare_before() {
    let Some((doc, original, start, len)) = BEFORE.with(|b| b.borrow().clone()) else { return };
    let current = docs::doc_text(doc);
    if start + len > current.len() {
        set_status("The document changed too much to rebuild the earlier text");
        return;
    }
    let mut before = Vec::with_capacity(current.len() + original.len());
    before.extend_from_slice(&current[..start]);
    before.extend_from_slice(&original);
    before.extend_from_slice(&current[start + len..]);
    compare::start_with_text(doc, "Before AI", before, true);
}

// ------------------------------------------------------------------ DPAPI and base64

mod dpapi {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};

    fn call(data: &[u8], protect: bool) -> Option<Vec<u8>> {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        let ok = unsafe {
            if protect {
                CryptProtectData(&input, windows_sys::w!("Nib"), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
            } else {
                CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
            }
        };
        if ok == 0 || out.pbData.is_null() {
            return None;
        }
        let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec() };
        unsafe { LocalFree(out.pbData as _) };
        Some(v)
    }

    pub fn protect(data: &[u8]) -> Option<Vec<u8>> {
        call(data, true)
    }
    pub fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
        call(data, false)
    }
}

mod base64 {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn encode(d: &[u8]) -> String {
        let mut s = String::new();
        for c in d.chunks(3) {
            let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
            for i in 0..4 {
                if i <= c.len() {
                    s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
                } else {
                    s.push('=');
                }
            }
        }
        s
    }

    pub fn decode(s: &str) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        let mut buf = 0u32;
        let mut bits = 0;
        for ch in s.bytes().filter(|b| !b.is_ascii_whitespace()) {
            if ch == b'=' {
                break;
            }
            let v = T.iter().position(|&t| t == ch)? as u32;
            buf = buf << 6 | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((buf >> bits) as u8);
            }
        }
        Some(out)
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn round_trip() {
            for s in [&b""[..], b"f", b"fo", b"foo", b"foobar", &[0u8, 255, 128, 7]] {
                assert_eq!(super::decode(&super::encode(s)).unwrap(), s);
            }
            assert_eq!(super::encode(b"foobar"), "Zm9vYmFy");
        }
    }
}
