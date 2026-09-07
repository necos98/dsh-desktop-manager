// Genera le icone Tauri dalla sorgente ufficiale ./icon.png, usata tale e quale.
// Uso: node tools/gen-icon.mjs (oppure npm run icons)
// Copia la PNG in src-tauri/icons (icon.png + 32x32/128x128/256x256) e crea
// icon.ico come ICO PNG-compressa (Vista+) a singola entry da 256px.
// Windows ridimensiona da solo per taskbar, barra del titolo e anteprime;
// Tauri fa lo stesso in fase di bundle. Zero dipendenze.
import { readFileSync, mkdirSync, writeFileSync } from "node:fs";
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

mkdirSync(outDir, { recursive: true });
writeFileSync(join(outDir, "icon.png"), png);
for (const s of [32, 128, 256]) writeFileSync(join(outDir, s + "x" + s + ".png"), png);
// Le PNG nominali riusano gli stessi byte della 256: Tauri e Windows ridimensionano da soli.

// ICO PNG-compressa, singola entry 256 (width/height 0 significa 256)
const header = Buffer.alloc(6);
header.writeUInt16LE(0, 0);
header.writeUInt16LE(1, 2);
header.writeUInt16LE(1, 4);
const entry = Buffer.alloc(16);
entry[0] = 0;
entry[1] = 0;
entry[2] = 0;
entry[3] = 0;
entry.writeUInt16LE(1, 4);
entry.writeUInt16LE(32, 6);
entry.writeUInt32LE(png.length, 8);
entry.writeUInt32LE(22, 12);
writeFileSync(join(outDir, "icon.ico"), Buffer.concat([header, entry, png]));

console.log("gen-icon: icone aggiornate da icon.png in", outDir);