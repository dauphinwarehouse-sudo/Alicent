// Minimal code-native brand mark. Writes a valid 32-bit 32x32 ICO without dependencies.
import { mkdirSync, writeFileSync } from 'node:fs';
const size = 32, pixels = Buffer.alloc(size * size * 4), mask = Buffer.alloc(size * 4);
for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) {
  const i = ((size - 1 - y) * size + x) * 4;
  const mark = y > 6 && y < 25 && (Math.abs(x - (16 - (y - 7) * .4)) < 1.5 || Math.abs(x - (16 + (y - 7) * .4)) < 1.5 || (y >= 18 && y <= 20 && x >= 12 && x <= 20));
  pixels.set(mark ? [255,255,255,255] : [153,89,32,255], i);
}
const dib = Buffer.alloc(40); dib.writeUInt32LE(40,0); dib.writeInt32LE(size,4); dib.writeInt32LE(size*2,8); dib.writeUInt16LE(1,12); dib.writeUInt16LE(32,14); dib.writeUInt32LE(pixels.length+mask.length,20);
const data = Buffer.concat([dib,pixels,mask]); const header = Buffer.alloc(22);
header.writeUInt16LE(1,2); header.writeUInt16LE(1,4); header[6]=size; header[7]=size;
header.writeUInt16LE(1,10); header.writeUInt16LE(32,12); header.writeUInt32LE(data.length,14); header.writeUInt32LE(22,18);
mkdirSync('apps/desktop/src-tauri/icons',{recursive:true});
writeFileSync('apps/desktop/src-tauri/icons/icon.ico',Buffer.concat([header,data]));
