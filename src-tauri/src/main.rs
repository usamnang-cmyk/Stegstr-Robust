use clap::{Parser, Subcommand};
use crc32fast::Hasher as Crc;
use image::{imageops, DynamicImage, GenericImageView, ImageOutputFormat, RgbImage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const MAGIC: &[u8; 8] = b"STEGSTR1";
const BLOCK: u32 = 8;
const DELTA: f32 = 12.0;
const REPEAT_FORTRESS: usize = 15;
const REPEAT_ARMOR_MAX: usize = 8;
const REPEAT_ARMOR: usize = 4;
const DERIVED_KEY: &[u8] = b"stegstr-robust-open-mode-v1";

#[derive(Parser)]
#[command(
    name = "stegstr-cli",
    version,
    about = "Stegstr Robust Edition — embed and detect payloads that survive social-media JPEG processing"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Embed a payload into a cover image
    Embed {
        cover: PathBuf,
        #[arg(short = 'o', long)]
        output: PathBuf,
        #[arg(long)]
        payload: String,
        #[arg(long, default_value = "robust")]
        mode: String,
        #[arg(long)]
        platform: Option<String>,
        #[arg(long)]
        encrypt: bool,
    },
    /// Decode + verify + print payload (app detect)
    Detect {
        image: PathBuf,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        decrypt: bool,
    },
    /// Decode raw payload bytes
    Decode { image: PathBuf },
    /// Create a Nostr-style kind-1 bundle
    Post {
        message: String,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        privkey_hex: Option<String>,
    },
    /// Resize / quality-target a carrier for a platform
    Prepare {
        image: PathBuf,
        #[arg(short = 'o', long)]
        output: PathBuf,
        #[arg(long, default_value = "whatsapp")]
        platform: String,
    },
    /// Simulate platform recompression + resize
    TestSurvival {
        image: PathBuf,
        #[arg(long)]
        simulate: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Estimate embed capacity
    Capacity {
        image: PathBuf,
        #[arg(long, default_value = "robust")]
        mode: String,
    },
}

#[derive(Clone, Copy)]
enum Mode {
    RobustFortress,
    ArmorMax,
    ArmorStandard,
    Lossless,
}

impl Mode {
    fn parse(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "robust" | "fortress" | "jpeg" => Ok(Mode::RobustFortress),
            "armor-max" | "armormax" => Ok(Mode::ArmorMax),
            "armor" | "armor-standard" => Ok(Mode::ArmorStandard),
            "lossless" | "png" => Ok(Mode::Lossless),
            other => Err(format!(
                "unknown mode '{other}'. Use robust, armor-max, armor-standard, or lossless"
            )),
        }
    }

    fn repeat(self) -> usize {
        match self {
            Mode::RobustFortress => REPEAT_FORTRESS,
            Mode::ArmorMax => REPEAT_ARMOR_MAX,
            Mode::ArmorStandard => REPEAT_ARMOR,
            Mode::Lossless => 1,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Mode::RobustFortress => "robust",
            Mode::ArmorMax => "armor-max",
            Mode::ArmorStandard => "armor-standard",
            Mode::Lossless => "lossless",
        }
    }

    fn is_jpeg(self) -> bool {
        !matches!(self, Mode::Lossless)
    }
}

#[derive(Serialize, Deserialize)]
struct Bundle {
    version: u32,
    events: Vec<serde_json::Value>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::from(1)
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Commands::Embed {
            cover,
            output,
            payload,
            mode,
            platform,
            encrypt,
        } => cmd_embed(&cover, &output, &payload, &mode, platform.as_deref(), encrypt),
        Commands::Detect {
            image,
            json,
            decrypt,
        } => cmd_detect(&image, json, decrypt),
        Commands::Decode { image } => cmd_decode(&image),
        Commands::Post {
            message,
            output,
            privkey_hex,
        } => cmd_post(&message, output.as_deref(), privkey_hex.as_deref()),
        Commands::Prepare {
            image,
            output,
            platform,
        } => cmd_prepare(&image, &output, &platform),
        Commands::TestSurvival {
            image,
            simulate,
            output,
        } => cmd_test_survival(&image, &simulate, &output),
        Commands::Capacity { image, mode } => cmd_capacity(&image, &mode),
    }
}

fn cmd_embed(
    cover: &Path,
    output: &Path,
    payload_arg: &str,
    mode_s: &str,
    platform: Option<&str>,
    encrypt: bool,
) -> Result<(), String> {
    let mode = Mode::parse(mode_s)?;
    let raw = load_payload(payload_arg)?;
    let body = if encrypt {
        xor_crypt(&raw, DERIVED_KEY)
    } else {
        raw
    };
    let packet = pack_payload(&body, encrypt)?;

    if mode.is_jpeg() {
        let img = load_image(cover)?;
        let prepared = prepare_image(img, platform.unwrap_or("whatsapp"));
        let available = block_count(&prepared);
        let mut mode = mode;
        let mut bits_needed = packet.len() * 8 * mode.repeat();
        if bits_needed > available {
            for candidate in [Mode::ArmorMax, Mode::ArmorStandard] {
                let need = packet.len() * 8 * candidate.repeat();
                if need <= available {
                    eprintln!(
                        "note: payload does not fit {} (needs {bits_needed} bits, {available} blocks); using {} instead",
                        mode.name(),
                        candidate.name()
                    );
                    mode = candidate;
                    bits_needed = need;
                    break;
                }
            }
        }
        if bits_needed > available {
            return Err(format!(
                "payload too large for JPEG modes: need {bits_needed} coded bits, image has {available} blocks. Use a larger cover, --mode lossless, or a shorter payload."
            ));
        }
        let stego = embed_ba_qim(&prepared, &packet, mode.repeat());
        save_jpeg(&stego, output, 90)?;
        println!(
            "Embedded successfully in {} mode. Estimated survival: {}.",
            mode.name(),
            survival_label(mode, platform)
        );
        println!("Wrote {}", output.display());
    } else {
        let img = load_image(cover)?;
        let stego = embed_lsb_png(&img, &packet)?;
        stego
            .save(output)
            .map_err(|e| format!("write PNG failed: {e}"))?;
        println!("Embedded successfully in lossless mode. Estimated survival: none on photo send (file/document only).");
        println!("Wrote {}", output.display());
    }
    Ok(())
}

fn cmd_detect(path: &Path, json: bool, decrypt: bool) -> Result<(), String> {
    let packet = extract_any(path)?;
    let (flags, body) = unpack_payload(&packet)?;
    let text_bytes = if decrypt || flags & 0x01 != 0 {
        xor_crypt(&body, DERIVED_KEY)
    } else {
        body
    };

    if json {
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&text_bytes) {
            println!("{}", serde_json::to_string_pretty(&v).unwrap());
        } else {
            let s = String::from_utf8_lossy(&text_bytes).to_string();
            let out = serde_json::json!({
                "ok": true,
                "mode": "detected",
                "payload": s,
                "bytes": text_bytes.len()
            });
            println!("{}", serde_json::to_string_pretty(&out).unwrap());
        }
    } else if let Ok(s) = String::from_utf8(text_bytes.clone()) {
        println!("{s}");
    } else {
        println!("base64:{}", b64(&text_bytes));
    }
    Ok(())
}

fn cmd_decode(path: &Path) -> Result<(), String> {
    let packet = extract_any(path)?;
    let (_flags, body) = unpack_payload(&packet)?;
    if let Ok(s) = String::from_utf8(body.clone()) {
        println!("{s}");
    } else {
        println!("base64:{}", b64(&body));
    }
    Ok(())
}

fn cmd_post(message: &str, output: Option<&Path>, privkey_hex: Option<&str>) -> Result<(), String> {
    let pubkey = match privkey_hex {
        Some(hex) => {
            if hex.len() != 64 {
                return Err("privkey-hex must be 64 hex characters".into());
            }
            let mut hasher = Sha256::new();
            hasher.update(hex.as_bytes());
            hasher.update(b":pubkey");
            hex::encode(hasher.finalize())
        }
        None => {
            let mut hasher = Sha256::new();
            hasher.update(message.as_bytes());
            hasher.update(b":anon");
            hex::encode(hasher.finalize())
        }
    };
    let created_at = 1735689600u64;
    let event = serde_json::json!({
        "kind": 1,
        "pubkey": pubkey,
        "created_at": created_at,
        "tags": [],
        "content": message,
    });
    let bundle = Bundle {
        version: 1,
        events: vec![event],
    };
    let pretty = serde_json::to_string_pretty(&bundle).unwrap();
    if let Some(path) = output {
        fs::write(path, &pretty).map_err(|e| format!("write bundle failed: {e}"))?;
        println!("Wrote {}", path.display());
    } else {
        println!("{pretty}");
    }
    Ok(())
}

fn cmd_prepare(path: &Path, output: &Path, platform: &str) -> Result<(), String> {
    let img = load_image(path)?;
    let prepared = prepare_image(img, platform);
    save_jpeg(&prepared, output, platform_quality(platform))?;
    println!("Prepared carrier for {platform}: {}", output.display());
    Ok(())
}

fn cmd_test_survival(path: &Path, simulate: &str, output: &Path) -> Result<(), String> {
    let img = load_image(path)?;
    let attacked = simulate_platform(img, simulate);
    save_jpeg(&attacked, output, platform_quality(simulate))?;
    println!("Simulated {simulate} processing. Wrote {}", output.display());
    Ok(())
}

fn cmd_capacity(path: &Path, mode_s: &str) -> Result<(), String> {
    let mode = Mode::parse(mode_s)?;
    let img = load_image(path)?;
    let prepared = if mode.is_jpeg() {
        prepare_image(img, "whatsapp")
    } else {
        img
    };
    let blocks = block_count(&prepared);
    let payload_bytes = if mode.is_jpeg() {
        // header 8+1+1+2+4 = 16 bytes overhead
        (blocks / (8 * mode.repeat())).saturating_sub(16)
    } else {
        let (w, h) = prepared.dimensions();
        ((w as usize * h as usize * 3) / 8).saturating_sub(16)
    };
    println!(
        "mode={}  blocks_or_pixels_units={}  usable_payload_bytes≈{}  repeat={}",
        mode.name(),
        blocks,
        payload_bytes,
        mode.repeat()
    );
    Ok(())
}

fn load_payload(arg: &str) -> Result<Vec<u8>, String> {
    if let Some(path) = arg.strip_prefix('@') {
        fs::read(path).map_err(|e| format!("cannot read payload file {path}: {e}"))
    } else {
        Ok(arg.as_bytes().to_vec())
    }
}

fn pack_payload(body: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
    if body.len() > 65535 {
        return Err("payload exceeds 65535 bytes".into());
    }
    let mut out = Vec::with_capacity(16 + body.len());
    out.extend_from_slice(MAGIC);
    out.push(1); // version
    out.push(if encrypt { 0x01 } else { 0x00 });
    out.extend_from_slice(&(body.len() as u16).to_be_bytes());
    out.extend_from_slice(body);
    let mut hasher = Crc::new();
    hasher.update(&out);
    out.extend_from_slice(&hasher.finalize().to_be_bytes());
    Ok(out)
}

fn unpack_payload(packet: &[u8]) -> Result<(u8, Vec<u8>), String> {
    if packet.len() < 16 {
        return Err("no Stegstr payload found (packet too short)".into());
    }
    if &packet[0..8] != MAGIC {
        return Err("no Stegstr payload found (bad magic)".into());
    }
    let version = packet[8];
    if version != 1 {
        return Err(format!("unsupported payload version {version}"));
    }
    let flags = packet[9];
    let len = u16::from_be_bytes([packet[10], packet[11]]) as usize;
    if packet.len() < 16 + len {
        return Err("truncated Stegstr payload".into());
    }
    let body = packet[12..12 + len].to_vec();
    let got = u32::from_be_bytes(
        packet[12 + len..16 + len]
            .try_into()
            .map_err(|_| "truncated crc")?,
    );
    let mut hasher = Crc::new();
    hasher.update(&packet[..12 + len]);
    if hasher.finalize() != got {
        return Err("payload CRC mismatch — image may have been processed too aggressively, or this is not a Stegstr Robust image".into());
    }
    Ok((flags, body))
}

fn extract_any(path: &Path) -> Result<Vec<u8>, String> {
    let img = load_image(path)?;
    // Try robust first (works on JPEG and on PNG that was converted), then LSB.
    if let Ok(p) = extract_ba_qim(&img, REPEAT_FORTRESS) {
        if unpack_payload(&p).is_ok() {
            return Ok(p);
        }
    }
    for r in [REPEAT_ARMOR_MAX, REPEAT_ARMOR] {
        if let Ok(p) = extract_ba_qim(&img, r) {
            if unpack_payload(&p).is_ok() {
                return Ok(p);
            }
        }
    }
    if let Ok(p) = extract_lsb_png(&img) {
        if unpack_payload(&p).is_ok() {
            return Ok(p);
        }
    }
    Err("no valid Stegstr Robust payload detected".into())
}

fn load_image(path: &Path) -> Result<DynamicImage, String> {
    image::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))
}

fn prepare_image(img: DynamicImage, platform: &str) -> DynamicImage {
    let rgb = img.to_rgb8();
    let (mut w, mut h) = rgb.dimensions();
    let max_side = platform_max_side(platform);
    let long = w.max(h);
    if long > max_side {
        let scale = max_side as f32 / long as f32;
        w = ((w as f32) * scale).round().max(8.0) as u32;
        h = ((h as f32) * scale).round().max(8.0) as u32;
    }
    // Align to 8×8 blocks.
    w = (w / BLOCK) * BLOCK;
    h = (h / BLOCK) * BLOCK;
    w = w.max(BLOCK * 16);
    h = h.max(BLOCK * 16);
    DynamicImage::ImageRgb8(imageops::resize(
        &rgb,
        w,
        h,
        imageops::FilterType::Lanczos3,
    ))
}

fn simulate_platform(img: DynamicImage, platform: &str) -> DynamicImage {
    let prepared = prepare_image(img, platform);
    let q = platform_quality(platform).saturating_sub(15).max(50);
    let mut buf = Vec::new();
    prepared
        .write_to(&mut Cursor::new(&mut buf), ImageOutputFormat::Jpeg(q))
        .ok();
    image::load_from_memory(&buf).unwrap_or(prepared)
}

fn platform_max_side(platform: &str) -> u32 {
    match platform.to_ascii_lowercase().as_str() {
        "instagram" => 1440,
        "telegram" => 2560,
        _ => 1600, // whatsapp default
    }
}

fn platform_quality(platform: &str) -> u8 {
    match platform.to_ascii_lowercase().as_str() {
        "instagram" => 70,
        "telegram" => 75,
        _ => 70,
    }
}

fn survival_label(mode: Mode, platform: Option<&str>) -> String {
    match mode {
        Mode::RobustFortress => format!(
            "high (Fortress, target {})",
            platform.unwrap_or("whatsapp")
        ),
        Mode::ArmorMax => "usually high with carrier prep".into(),
        Mode::ArmorStandard => "moderate — milder platforms".into(),
        Mode::Lossless => "none on photo recompression".into(),
    }
}

fn block_count(img: &DynamicImage) -> usize {
    let (w, h) = img.dimensions();
    ((w / BLOCK) * (h / BLOCK)) as usize
}

fn embed_ba_qim(img: &DynamicImage, packet: &[u8], repeat: usize) -> DynamicImage {
    let mut rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();
    let bw = w / BLOCK;
    let bh = h / BLOCK;
    let bits = bytes_to_bits(packet);
    let mut coded = Vec::with_capacity(bits.len() * repeat);
    for bit in bits {
        for _ in 0..repeat {
            coded.push(bit);
        }
    }
    let mut i = 0usize;
    for by in 0..bh {
        for bx in 0..bw {
            if i >= coded.len() {
                break;
            }
            qim_block(&mut rgb, bx * BLOCK, by * BLOCK, coded[i]);
            i += 1;
        }
        if i >= coded.len() {
            break;
        }
    }
    DynamicImage::ImageRgb8(rgb)
}

fn extract_ba_qim(img: &DynamicImage, repeat: usize) -> Result<Vec<u8>, String> {
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();
    let bw = w / BLOCK;
    let bh = h / BLOCK;
    let mut raw_bits = Vec::new();
    for by in 0..bh {
        for bx in 0..bw {
            raw_bits.push(read_qim_block(&rgb, bx * BLOCK, by * BLOCK));
        }
    }
    if raw_bits.len() < 16 * 8 * repeat {
        return Err("image too small".into());
    }
    let mut bits = Vec::new();
    for chunk in raw_bits.chunks(repeat) {
        let ones = chunk.iter().filter(|b| **b).count();
        bits.push(ones * 2 >= chunk.len());
        if bits.len() >= 16 * 8 {
            // we don't yet know payload length; keep going a bit later
        }
    }
    // Need header first: 12 header bytes + later length
    if bits.len() < 12 * 8 {
        return Err("not enough bits".into());
    }
    let header = bits_to_bytes(&bits[..12 * 8]);
    if header.len() < 12 || &header[0..8] != MAGIC {
        return Err("bad magic".into());
    }
    let payload_len = u16::from_be_bytes([header[10], header[11]]) as usize;
    let total = 16 + payload_len;
    let need_bits = total * 8;
    if bits.len() < need_bits {
        return Err("truncated coded payload".into());
    }
    Ok(bits_to_bytes(&bits[..need_bits]))
}

fn qim_block(img: &mut RgbImage, x0: u32, y0: u32, bit: bool) {
    let mut sum = 0.0f32;
    let mut n = 0.0f32;
    for y in y0..y0 + BLOCK {
        for x in x0..x0 + BLOCK {
            let p = img.get_pixel(x, y);
            // Rec. 601 luma
            let yv = 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32;
            sum += yv;
            n += 1.0;
        }
    }
    let avg = sum / n;
    let d = if bit { DELTA / 2.0 } else { 0.0 };
    let k = ((avg - d) / DELTA).round();
    let target = k * DELTA + d;
    let shift = (target - avg).clamp(-24.0, 24.0);
    for y in y0..y0 + BLOCK {
        for x in x0..x0 + BLOCK {
            let p = img.get_pixel_mut(x, y);
            for c in 0..3 {
                let v = p[c] as f32 + shift;
                p[c] = v.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

fn read_qim_block(img: &RgbImage, x0: u32, y0: u32) -> bool {
    let mut sum = 0.0f32;
    let mut n = 0.0f32;
    for y in y0..y0 + BLOCK {
        for x in x0..x0 + BLOCK {
            if x >= img.width() || y >= img.height() {
                continue;
            }
            let p = img.get_pixel(x, y);
            let yv = 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32;
            sum += yv;
            n += 1.0;
        }
    }
    let avg = if n > 0.0 { sum / n } else { 0.0 };
    let r = avg.rem_euclid(DELTA);
    (r - DELTA / 2.0).abs() < (r).min((DELTA - r).abs())
}

fn embed_lsb_png(img: &DynamicImage, packet: &[u8]) -> Result<DynamicImage, String> {
    let mut rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();
    let cap = (w as usize) * (h as usize) * 3;
    let bits = bytes_to_bits(packet);
    if bits.len() > cap {
        return Err("payload too large for lossless capacity".into());
    }
    let mut i = 0usize;
    'outer: for y in 0..h {
        for x in 0..w {
            let p = rgb.get_pixel_mut(x, y);
            for c in 0..3 {
                if i >= bits.len() {
                    break 'outer;
                }
                p[c] = (p[c] & 0xFE) | u8::from(bits[i]);
                i += 1;
            }
        }
    }
    Ok(DynamicImage::ImageRgb8(rgb))
}

fn extract_lsb_png(img: &DynamicImage) -> Result<Vec<u8>, String> {
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();
    let mut bits = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let p = rgb.get_pixel(x, y);
            for c in 0..3 {
                bits.push(p[c] & 1 == 1);
            }
        }
    }
    if bits.len() < 12 * 8 {
        return Err("too few LSB bits".into());
    }
    let header = bits_to_bytes(&bits[..12 * 8]);
    if header.len() < 12 || &header[0..8] != MAGIC {
        return Err("bad lsb magic".into());
    }
    let payload_len = u16::from_be_bytes([header[10], header[11]]) as usize;
    let total = 16 + payload_len;
    if bits.len() < total * 8 {
        return Err("truncated lsb payload".into());
    }
    Ok(bits_to_bytes(&bits[..total * 8]))
}

fn bytes_to_bits(data: &[u8]) -> Vec<bool> {
    let mut bits = Vec::with_capacity(data.len() * 8);
    for b in data {
        for i in (0..8).rev() {
            bits.push((b >> i) & 1 == 1);
        }
    }
    bits
}

fn bits_to_bytes(bits: &[bool]) -> Vec<u8> {
    bits.chunks(8)
        .map(|c| {
            let mut v = 0u8;
            for (i, bit) in c.iter().enumerate() {
                if *bit {
                    v |= 1 << (7 - i);
                }
            }
            v
        })
        .collect()
}

fn xor_crypt(data: &[u8], key: &[u8]) -> Vec<u8> {
    data.iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % key.len()])
        .collect()
}

fn save_jpeg(img: &DynamicImage, path: &Path, quality: u8) -> Result<(), String> {
    let rgb = img.to_rgb8();
    let mut buf = Vec::new();
    DynamicImage::ImageRgb8(rgb)
        .write_to(&mut Cursor::new(&mut buf), ImageOutputFormat::Jpeg(quality))
        .map_err(|e| format!("jpeg encode failed: {e}"))?;
    fs::write(path, buf).map_err(|e| format!("write {} failed: {e}", path.display()))
}

fn b64(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let a = chunk[0] as u32;
        let b = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let c = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (a << 16) | (b << 8) | c;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(T[((n >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(T[(n & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}
