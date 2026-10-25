// 生成 Pulse 应用图标源图（1024x1024 PNG，纯 Node 实现，无图像库依赖）
// 设计：深色圆角方形 + 绿色电量环（72%）+ 白色闪电
import { deflateSync } from 'node:zlib';
import { writeFileSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const SIZE = 1024;
const SS = 2; // 2x 超采样抗锯齿
const N = SIZE * SS;

// ---------- PNG 编码 ----------
function makeCrcTable() {
  const t = new Int32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c;
  }
  return t;
}
const CRC_TABLE = makeCrcTable();
function crc32(buf) {
  let crc = -1;
  for (let i = 0; i < buf.length; i++) crc = (crc >>> 8) ^ CRC_TABLE[(crc ^ buf[i]) & 0xff];
  return (crc ^ -1) >>> 0;
}
function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const td = Buffer.concat([Buffer.from(type, 'ascii'), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(td));
  return Buffer.concat([len, td, crc]);
}

// ---------- 绘图 ----------
const cx = N / 2, cy = N / 2;
const R = N * (340 / 1024);   // 环半径
const HW = N * (38 / 1024);   // 环半宽
const ARC = 0.72;             // 电量弧占比
const CORNER = N * (232 / 1024);

const BG0 = [13, 20, 36];    // #0D1424
const BG1 = [7, 11, 20];     // #070B14
const TRACK = [38, 50, 74];  // #26324A
const GREEN = [74, 222, 128];// #4ADE80
const WHITE = [241, 245, 251];

const BOLT = [[565, 290], [400, 560], [495, 560], [455, 750], [640, 460], [540, 460]]
  .map(([x, y]) => [x / 1024 * N, y / 1024 * N]);

function inRoundedRect(x, y) {
  const r = CORNER;
  const minX = r, maxX = N - r, minY = r, maxY = N - r;
  if (x < 0 || y < 0 || x >= N || y >= N) return false;
  const cxq = Math.min(Math.max(x, minX), maxX);
  const cyq = Math.min(Math.max(y, minY), maxY);
  const dx = x - cxq, dy = y - cyq;
  return dx * dx + dy * dy <= r * r;
}

function inPoly(px, py, poly) {
  let inside = false;
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
    const [xi, yi] = poly[i], [xj, yj] = poly[j];
    if (yi > py !== yj > py && px < ((xj - xi) * (py - yi)) / (yj - yi) + xi) inside = !inside;
  }
  return inside;
}

const pixels = Buffer.alloc(N * N * 4);
for (let y = 0; y < N; y++) {
  for (let x = 0; x < N; x++) {
    let color;
    if (inPoly(x, y, BOLT)) {
      color = WHITE;
    } else {
      const d = Math.hypot(x - cx, y - cy);
      if (Math.abs(d - R) <= HW) {
        // 顶部顺时针
        const theta = ((Math.atan2(x - cx, cy - y) * 180) / Math.PI + 360) % 360;
        color = theta <= ARC * 360 ? GREEN : TRACK;
      } else {
        const t = (x + y) / (2 * N);
        color = [
          Math.round(BG0[0] + (BG1[0] - BG0[0]) * t),
          Math.round(BG0[1] + (BG1[1] - BG0[1]) * t),
          Math.round(BG0[2] + (BG1[2] - BG0[2]) * t),
        ];
      }
    }
    const alpha = inRoundedRect(x, y) ? 255 : 0;
    const o = (y * N + x) * 4;
    pixels[o] = color[0];
    pixels[o + 1] = color[1];
    pixels[o + 2] = color[2];
    pixels[o + 3] = alpha;
  }
}

// 2x 下采样
const out = Buffer.alloc(SIZE * SIZE * 4);
for (let y = 0; y < SIZE; y++) {
  for (let x = 0; x < SIZE; x++) {
    const ox = x * SS, oy = y * SS;
    let r = 0, g = 0, b = 0, a = 0;
    for (let dy = 0; dy < SS; dy++)
      for (let dx = 0; dx < SS; dx++) {
        const o = ((oy + dy) * N + ox + dx) * 4;
        r += pixels[o]; g += pixels[o + 1]; b += pixels[o + 2]; a += pixels[o + 3];
      }
    const c = SS * SS;
    const o2 = (y * SIZE + x) * 4;
    out[o2] = r / c; out[o2 + 1] = g / c; out[o2 + 2] = b / c; out[o2 + 3] = a / c;
  }
}

// 组装 PNG
const raw = Buffer.alloc(SIZE * (SIZE * 4 + 1));
for (let y = 0; y < SIZE; y++) {
  raw[y * (SIZE * 4 + 1)] = 0; // filter: None
  out.copy(raw, y * (SIZE * 4 + 1) + 1, y * SIZE * 4, (y + 1) * SIZE * 4);
}
const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(SIZE, 0);
ihdr.writeUInt32BE(SIZE, 4);
ihdr[8] = 8; ihdr[9] = 6; // 8-bit RGBA
const png = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  chunk('IHDR', ihdr),
  chunk('IDAT', deflateSync(raw, { level: 9 })),
  chunk('IEND', Buffer.alloc(0)),
]);

const dir = join(__dirname, '..', 'app-assets');
mkdirSync(dir, { recursive: true });
writeFileSync(join(dir, 'app-icon.png'), png);
console.log(`app-icon.png written (${png.length} bytes, ${SIZE}x${SIZE})`);
