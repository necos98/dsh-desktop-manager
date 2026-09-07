// Genera le icone Tauri dalla sorgente ufficiale ./icon.png, usata tale e quale.
// Uso: node tools/gen-icon.mjs (oppure npm run icons)
// - Copia la PNG in src-tauri/icons (icon.png + 32x32/128x128/256x256).
// - Crea icon.ico come ICO PNG-compressa (Vista+) MULTI-SIZE: Windows sceglie
//   l'entry giusta per taskbar (24/32/40/48), barra titolo (16), Alt+Tab e
//   anteprime, invece di scalare una sola 256 (che lascia l'icona vecchia).
//   Le entry piccole sono ricampionate dalla 256 con filtro box gamma-corrected.
// Zero dipendenze (solo node:zlib per ricomprimere le PNG ridotte).
import { readFileSync, mkdirSync, writeFileSync } from "node:fs";
import { inflateSync, deflateSync } from "node:zlib";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const src = join(root, "icon.png");
const outDir = join(root, "src-tauri", "icons");

const png = readFileSync(src);
const sig = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
for (let i = 0; i < sig.length; i++) {
  if (png[i] !== sig[i]) {
    console.error("gen-icon: errore: icon.png non valida (firma PNG assente)");
    process.exit(1);
  }
}

// ---- PNG decoder minimo: serve RGBA 8bit non-interlacciato (come icon.png) ----
function u32(b, o) {
  return ((b[o] << 24) | (b[o + 1] << 16) | (b[o + 2] << 8) | b[o + 3]) >>> 0;
}
const crcTable = (() => {
  const t = new Int32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c;
  }
  return t;
})();
function crc32(buf) {
  let c = -1;
  for (let i = 0; i < buf.length; i++) c = crcTable[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ -1) >>> 0;
}
function pngChunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}
function paeth(a, b, c) {
  const p = a + b - c;
  const pa = Math.abs(p - a);
  const pb = Math.abs(p - b);
  const pc = Math.abs(p - c);
  return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
}
function decodePngRgba(buf) {
  let o = 8;
  let w = 0;
  let h = 0;
  let bitDepth = 0;
  let colorType = 0;
  const idat = [];
  while (o < buf.length) {
    const len = u32(buf, o);
    const type = buf.toString("ascii", o + 4, o + 8);
    const data = buf.subarray(o + 8, o + 8 + len);
    if (type === "IHDR") {
      w = u32(data, 0);
      h = u32(data, 4);
      bitDepth = data[8];
      colorType = data[9];
      if (data[12] !== 0) throw new Error('interlacciato non supportato');
    } else if (type === "IDAT") {
      idat.push(Buffer.from(data));
    }
    o += 12 + len;
  }
  if (colorType !== 6 || bitDepth !== 8) throw new Error('serve PNG RGBA 8-bit, converti icon.png');
  const raw = inflateSync(Buffer.concat(idat));
  const rgba = Buffer.alloc(w * h * 4);
  const bpp = 4;
  let p = 0;
  for (let y = 0; y < h; y++) {
    const f = raw[p++];
    for (let x = 0; x < w; x++) {
      for (let c = 0; c < bpp; c++) {
        let v = raw[p++];
        const i = (y * w + x) * 4 + c;
        const a = x < bpp ? 0 : rgba[i - bpp];
        const b = y === 0 ? 0 : rgba[i - w * bpp];
        const cc = x < bpp || y === 0 ? 0 : rgba[i - w * bpp - bpp];
        if (f === 1) v = (v + a) & 255;
        else if (f === 2) v = (v + b) & 255;
        else if (f === 3) v = (v + ((a + b) >> 1)) & 255;
        else if (f === 4) v = (v + paeth(a, b, cc)) & 255;
        rgba[i] = v;
      }
    }
  }
  return { w, h, rgba };
}
function encodePngRgba(w, h, rgba) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8;
  ihdr[9] = 6;
  const raw = Buffer.alloc((w * 4 + 1) * h);
  for (let y = 0; y < h; y++) {
    raw[y * (w * 4 + 1)] = 0;
    rgba.copy(raw, y * (w * 4 + 1) + 1, y * w * 4, (y + 1) * w * 4);
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    pngChunk("IHDR", ihdr),
    pngChunk("IDAT", deflateSync(raw, { level: 9 })),
    pngChunk("IEND", Buffer.alloc(0)),
  ]);
}

// ---- Ricampionamento box con gamma 2.2: evita aloni sui bordi alpha ----
function toLin(v) { return Math.pow(v / 255, 2.2); }
function toSrgb(v) { return Math.round(255 * Math.pow(Math.max(0, Math.min(1, v)), 1 / 2.2)); }
function downscale(srcImg, dst) {
  const out = Buffer.alloc(dst * dst * 4);
  const k = srcImg.w / dst;
  for (let y = 0; y < dst; y++) {
    for (let x = 0; x < dst; x++) {
      let r = 0;
      let g = 0;
      let b = 0;
      let a = 0;
      const x0 = Math.floor(x * k);
      const x1 = Math.max(x0 + 1, Math.floor((x + 1) * k));
      const y0 = Math.floor(y * k);
      const y1 = Math.max(y0 + 1, Math.floor((y + 1) * k));
      let n = 0;
      for (let sy = y0; sy < y1; sy++) {
        for (let sx = x0; sx < x1; sx++) {
          const i = (sy * srcImg.w + sx) * 4;
          const sa = srcImg.rgba[i + 3] / 255;
          r += toLin(srcImg.rgba[i]) * sa;
          g += toLin(srcImg.rgba[i + 1]) * sa;
          b += toLin(srcImg.rgba[i + 2]) * sa;
          a += sa;
          n++;
        }
      }
      const i = (y * dst + x) * 4;
      if (a > 0) {
        out[i] = toSrgb(r / a);
        out[i + 1] = toSrgb(g / a);
        out[i + 2] = toSrgb(b / a);
        out[i + 3] = Math.round((255 * a) / n);
      }
    }
  }
  return encodePngRgba(dst, dst, out);
}

mkdirSync(outDir, { recursive: true });
writeFileSync(join(outDir, "icon.png"), png);
const srcImg = decodePngRgba(png);
console.log("gen-icon: sorgente " + srcImg.w + "x" + srcImg.h);
const rendered = new Map();
rendered.set(256, png);
for (const s of [16, 24, 32, 48, 64, 128]) {
  if (srcImg.w === s && srcImg.h === s) rendered.set(s, png);
  else rendered.set(s, downscale(srcImg, s));
}
for (const [s, buf] of rendered) writeFileSync(join(outDir, s + "x" + s + ".png"), buf);

// ICO PNG-compressa: una entry per misura (0 = 256)
const sizes = [16, 24, 32, 48, 64, 128, 256];
const header = Buffer.alloc(6);
header.writeUInt16LE(0, 0);
header.writeUInt16LE(1, 2);
header.writeUInt16LE(sizes.length, 4);
const dir = Buffer.alloc(16 * sizes.length);
let off = 6 + dir.length;
const parts = [header, dir];
sizes.forEach((s, i) => {
  const data = rendered.get(s);
  const e = i * 16;
  dir[e] = s >= 256 ? 0 : s;
  dir[e + 1] = s >= 256 ? 0 : s;
  dir[e + 2] = 0;
  dir[e + 3] = 0;
  dir.writeUInt16LE(1, e + 4);
  dir.writeUInt16LE(32, e + 6);
  dir.writeUInt32LE(data.length, e + 8);
  dir.writeUInt32LE(off, e + 12);
  off += data.length;
  parts.push(data);
});
writeFileSync(join(outDir, "icon.ico"), Buffer.concat(parts));

console.log("gen-icon: icon.ico multi-size (" + sizes.join(",") + ") da icon.png in", outDir);