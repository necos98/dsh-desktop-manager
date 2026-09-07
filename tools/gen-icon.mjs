// Genera le icone del progetto (icons/icon.ico + PNG) senza dipendenze esterne.
// Crea un'icona 256x256 stilizzata: balena bianca in stile emoji su sfondo oceano.
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

// ---- Sfondo: quadrato arrotondato con gradiente oceano ----
const px = Buffer.alloc(SIZE * SIZE * 4);
const CORNER = 44;
function inRoundRect(x, y) {
  const dx = Math.max(CORNER - x, x - (SIZE - 1 - CORNER), 0);
  const dy = Math.max(CORNER - y, y - (SIZE - 1 - CORNER), 0);
  return Math.sqrt(dx * dx + dy * dy) <= CORNER;
}
function bgAt(y) {
  const t = y / SIZE;
  return [
    Math.round(79 + (124 - 79) * t),
    Math.round(140 + (92 - 140) * t),
    255,
  ];
}
function setPx(x, y, r, g, b, a = 255) {
  if (x < 0 || y < 0 || x >= SIZE || y >= SIZE) return;
  if (!inRoundRect(x, y)) return; // resta fuori dal quadrato arrotondato
  const i = (y * SIZE + x) * 4;
  px[i] = r;
  px[i + 1] = g;
  px[i + 2] = b;
  px[i + 3] = a;
}

for (let y = 0; y < SIZE; y++) {
  for (let x = 0; x < SIZE; x++) {
    const [r, g, b] = bgAt(y);
    setPx(x, y, r, g, b);
  }
}

// ---- Balena in stile emoji: vista laterale, guarda a sinistra ----
const WHITE = [255, 255, 255];
const BELLY = [205, 233, 255];
const FIN = [150, 205, 255];
const DARK = [25, 55, 95];
const SPOUT = [215, 240, 255];

function fillEllipse(cx, cy, rx, ry, c) {
  for (let y = Math.floor(cy - ry); y <= Math.ceil(cy + ry); y++) {
    for (let x = Math.floor(cx - rx); x <= Math.ceil(cx + rx); x++) {
      const dx = (x - cx) / rx;
      const dy = (y - cy) / ry;
      if (dx * dx + dy * dy <= 1) setPx(x, y, c[0], c[1], c[2]);
    }
  }
}

function fillCircle(cx, cy, r, color) {
  fillEllipse(cx, cy, r, r, color);
}

function dot(x, y, w, c) {
  const h = Math.floor(w / 2);
  for (let j = y - h; j <= y + h; j++) {
    for (let i = x - h; i <= x + h; i++) setPx(i, j, c[0], c[1], c[2]);
  }
}

function bodyHit(x, y) {
  const dx = (x - 124) / 96;
  const dy = (y - 158) / 60;
  return dx * dx + dy * dy <= 1;
}

function drawWhale() {
  // onda di schiuma in basso
  for (let y = 234; y < SIZE; y++) {
    for (let x = 0; x < SIZE; x++) {
      const dx = (x - 128) / 150;
      const dy = (y - 272) / 30;
      if (dx * dx + dy * dy <= 1) setPx(x, y, SPOUT[0], SPOUT[1], SPOUT[2]);
    }
  }

  // peduncolo caudale
  for (let t = 0; t <= 1.001; t += 0.02) {
    fillCircle(
      Math.round(200 + (232 - 200) * t),
      Math.round(150 + (124 - 150) * t),
      15,
      WHITE,
    );
  }
  // lobi della pinna caudale
  fillEllipse(246, 100, 22, 14, WHITE);
  fillEllipse(242, 124, 18, 10, WHITE);
  // intaglio a V tra i lobi
  for (let y = 100; y <= 126; y++) {
    for (let x = 236; x < SIZE; x++) {
      if (Math.abs(y - 113) <= (x - 236) * 0.45) {
        const [r, g, b] = bgAt(y);
        setPx(x, y, r, g, b);
      }
    }
  }

  // corpo
  fillEllipse(124, 158, 96, 60, WHITE);

  // ventre azzurro (ritagliato al corpo)
  for (let y = 154; y <= 210; y++) {
    for (let x = 52; x <= 180; x++) {
      const dx = (x - 116) / 64;
      const dy = (y - 182) / 30;
      if (dx * dx + dy * dy <= 1 && bodyHit(x, y)) {
        setPx(x, y, BELLY[0], BELLY[1], BELLY[2]);
      }
    }
  }

  // pinna pettorale
  for (let y = 174; y <= 196; y++) {
    for (let x = 110; x <= 154; x++) {
      const dx = (x - 132) / 22;
      const dy = (y - 186) / 10;
      if (dx * dx + dy * dy <= 1 && bodyHit(x, y)) {
        setPx(x, y, FIN[0], FIN[1], FIN[2]);
      }
    }
  }

  // occhio + riflesso
  fillCircle(70, 140, 8, DARK);
  fillCircle(67, 137, 3, WHITE);

  // sorriso
  for (let a = 10; a <= 100; a += 2) {
    const rad = (a * Math.PI) / 180;
    dot(
      Math.round(64 + 16 * Math.cos(rad)),
      Math.round(152 + 11 * Math.sin(rad)),
      6,
      DARK,
    );
  }

  // sfiatatoio
  fillCircle(96, 97, 4, DARK);

  // zampillo: colonna + gocce
  for (let y = 52; y <= 93; y++) dot(96, y, 8, SPOUT);
  for (let y = 58; y <= 93; y++) dot(96, y, 3, WHITE);
  fillCircle(96, 42, 9, SPOUT);
  fillCircle(96, 42, 4, WHITE);
  fillCircle(82, 56, 6, SPOUT);
  fillCircle(110, 56, 6, SPOUT);
}

drawWhale();

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
  // per i PNG nominali riusiamo la 256 (Tauri li ridimensiona in bundle; qui servono solo come placeholder validi)
  writeFileSync(join(outDir, s + 'x' + s + '.png'), png);
}

console.log("Icone generate in", outDir);