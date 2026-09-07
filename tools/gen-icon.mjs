// Genera le icone del progetto (icons/icon.ico + PNG) senza dipendenze esterne.
// Crea un'icona 256x256 stilizzata: gradiente + "D".
// Uso: node tools/gen-icon.mjs
import { deflateSync } from "node:zlib";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const outDir = join(root, "src-tauri", "icons");
mkdirSync(outDir, { recursive: true });

const SIZE = 256;

// ---- PNG encoder minimale (RGBA, niente filtri) ----
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

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

function encodePng(width, height, rgba) {
  const sig = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // color type RGBA
  const raw = Buffer.alloc((width * 4 + 1) * height);
  for (let y = 0; y < height; y++) {
    raw[y * (width * 4 + 1)] = 0; // filtro none
    rgba.copy(raw, y * (width * 4 + 1) + 1, y * width * 4, (y + 1) * width * 4);
  }
  return Buffer.concat([
    sig,
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

// ---- Disegno: gradiente verticale accent -> viola, monogramma "D" ----
const px = Buffer.alloc(SIZE * SIZE * 4);
function setPx(x, y, r, g, b, a = 255) {
  if (x < 0 || y < 0 || x >= SIZE || y >= SIZE) return;
  const i = (y * SIZE + x) * 4;
  px[i] = r;
  px[i + 1] = g;
  px[i + 2] = b;
  px[i + 3] = a;
}

// quadrato arrotondato di sfondo
for (let y = 0; y < SIZE; y++) {
  for (let x = 0; x < SIZE; x++) {
    const t = y / SIZE;
    const r = Math.round(79 + (124 - 79) * t);
    const g = Math.round(140 + (92 - 140) * t);
    const b = Math.round(255 + (255 - 255) * t);
    const corner = 44;
    const dx = Math.max(corner - x, x - (SIZE - 1 - corner), 0);
    const dy = Math.max(corner - y, y - (SIZE - 1 - corner), 0);
    const d = Math.sqrt(dx * dx + dy * dy);
    if (d > corner) continue; // fuori dal quadrato arrotondato
    setPx(x, y, r, g, b);
  }
}

// monogramma "D": gamba verticale sinistra + corpo ad arco (placeholder pulito)
const INK = [255, 255, 255];
function drawD() {
  const c = SIZE / 2;
  // gamba verticale sinistra
  for (let y = c - 92; y <= c + 92; y++) {
    for (let x = c - 58; x <= c - 34; x++) {
      setPx(Math.round(x), Math.round(y), INK[0], INK[1], INK[2]);
    }
  }
  // corpo: anello spesso tra raggio interno ed esterno
  for (let y = c - 92; y <= c + 92; y++) {
    for (let x = c - 34; x <= c + 100; x++) {
      const dist = Math.sqrt((x - c) ** 2 + (y - c) ** 2);
      if (dist <= 100 && dist >= 46) {
        setPx(Math.round(x), Math.round(y), INK[0], INK[1], INK[2]);
      }
    }
  }
}

drawD();

const png = encodePng(SIZE, SIZE, px);

// ICO (formato PNG-compressed, Vista+): header 6 + entry 16 + png
function icoFromPng(pngData, size) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0); // reserved
  header.writeUInt16LE(1, 2); // type: icon
  header.writeUInt16LE(1, 4); // count
  const entry = Buffer.alloc(16);
  entry[0] = size >= 256 ? 0 : size; // width
  entry[1] = size >= 256 ? 0 : size; // height
  entry[2] = 0; // colors
  entry[3] = 0; // reserved
  entry.writeUInt16LE(1, 4); // planes
  entry.writeUInt16LE(32, 6); // bit count
  entry.writeUInt32LE(pngData.length, 8); // size
  entry.writeUInt32LE(22, 12); // offset
  return Buffer.concat([header, entry, pngData]);
}

// icona 256 -> ICO entry size 0
const ico = icoFromPng(png, 256);

// Sizes richiesti da Tauri per i vari target
writeFileSync(join(outDir, "icon.ico"), ico);
writeFileSync(join(outDir, "icon.png"), png);
for (const s of [32, 128, 256]) {
  // per i PNG "nominali" riusiamo la 256 (Tauri li ridimensiona in bundle; qui servono solo come placeholder validi)
  writeFileSync(join(outDir, `${s}x${s}.png`), png);
}
writeFileSync(join(outDir, "icon.png"), png);

console.log("Icone generate in", outDir);
