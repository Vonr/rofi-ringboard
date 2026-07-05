use anyhow::{Result, bail};
use base64::Engine;
use bstr::ByteSlice;
use miniserde::json::{Object, Value};
use std::{
    io::Write,
    process::{Command, Stdio},
};

fn main() -> Result<()> {
    let out = Command::new("ringboard").args(["debug", "dump"]).output()?;
    let mut dump = if out.status.success() {
        let stdout = String::from_utf8(out.stdout)?;

        let dump: Vec<Object> = miniserde::json::from_str(&stdout)?;
        let mut dump = dump
            .into_iter()
            .map(|obj| {
                let mut data = None;
                let mut id = None;
                for (k, v) in obj.into_iter() {
                    if k == "id"
                        && let Value::Number(n) = v
                    {
                        id = Some(n);
                    } else if k == "data"
                        && let Value::String(s) = v
                    {
                        data = Some(s);
                    }
                }

                let id = match id.expect("entries should all have an id") {
                    miniserde::json::Number::U64(id) => Ok(id),
                    miniserde::json::Number::I64(id) => id.try_into(),
                    miniserde::json::Number::F64(id) => unreachable!("id {id} should not be float"),
                }
                .expect("id should be valid");

                let data = data.expect("entries should all have data");

                (id, data)
            })
            .collect::<Vec<_>>();
        dump.sort_by_key(|e| !e.0);
        dump
    } else {
        Vec::new()
    };

    let mut rofi = Command::new("rofi")
        .args(["-dmenu", "-sep", r"\0", "-p", "Ringboard", "-format", "i"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    let mut stdin = rofi.stdin.take().unwrap();

    let mut buf = Vec::new();
    for (_, data) in &dump {
        buf.clear();
        if base64::engine::general_purpose::STANDARD_NO_PAD
            .decode_vec(data, &mut buf)
            .is_ok()
            && let Some(ty) = infer::get(&buf)
            && ty.mime_type() != "text/plain"
        {
            write!(stdin, "<{}> ({} bytes)\0", ty, buf.len())?;
            continue;
        }

        buf.clear();
        let bytes = data.as_bytes();
        let start = bytes
            .iter()
            .copied()
            .position(|b| !b.is_ascii_whitespace())
            .unwrap_or(0);
        bytes[start..].replace_into(b"\n", b"\\n", &mut buf);
        buf.push(b'\0');
        stdin.write_all(&buf)?;
    }

    let mut output = rofi.wait_with_output()?.stdout;
    if output.is_empty() {
        bail!("rofi terminated");
    }
    output.remove(output.len() - 1);
    let mut id: u64 = 0;
    for n in output {
        if !n.is_ascii_digit() {
            unreachable!("invalid rofi output")
        }

        id = id * 10 + (n & 0xf) as u64;
    }

    let data = std::mem::take(&mut dump[id as usize].1);
    let mut mime_type = "text/plain";

    buf.clear();
    if base64::engine::general_purpose::STANDARD_NO_PAD
        .decode_vec(&data, &mut buf)
        .is_ok()
        && let Some(ty) = infer::get(&buf)
        && ty.mime_type() != "text/plain"
    {
        mime_type = ty.mime_type();
    } else {
        buf.clear();
        buf.extend_from_slice(data.as_bytes());
    }

    let mut copy = Command::new("ringboard")
        .args(["add", "-m", mime_type, "-c", "-"])
        .stdout(Stdio::null())
        .stdin(Stdio::piped())
        .spawn()?;
    copy.stdin.take().unwrap().write_all(&buf)?;
    copy.wait()?;

    Command::new("ringboard")
        .args(["remove", &dump[id as usize].0.to_string()])
        .stdout(Stdio::null())
        .spawn()?
        .wait()?;

    Ok(())
}
