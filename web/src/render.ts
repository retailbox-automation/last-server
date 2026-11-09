// Draws a DiveView on a canvas with plain shapes and colours (no external art).

import { ITEM_COLOR, tokenOf } from './items.ts';
import type { DiveView } from './sim.ts';
import { TILE } from './sim.ts';

export const TILE_PX = 20;

const C = {
  wall: '#262f45',
  wallEdge: '#1b2233',
  floor: '#4a5675',
  floorAlt: '#444f6d',
  stairs: '#2f9e6f',
  door: '#2d5fd0',
  player: '#ffd166',
  playerEdge: '#2a2113',
  monster: '#e5484d',
  grave: '#a7b0c2',
  graveLooted: '#5a6378',
  exit: '#7dd3fc',
};

export function setupCanvas(canvas: HTMLCanvasElement, tiles: number): CanvasRenderingContext2D {
  canvas.width = tiles * TILE_PX;
  canvas.height = tiles * TILE_PX;
  const ctx = canvas.getContext('2d');
  if (!ctx) throw new Error('Canvas is not available in this browser.');
  return ctx;
}

function drawItem(ctx: CanvasRenderingContext2D, token: string, px: number, py: number): void {
  const color = ITEM_COLOR[token] ?? '#ffffff';
  ctx.fillStyle = color;
  ctx.strokeStyle = '#0b1020';
  ctx.lineWidth = 1;
  const cx = px + TILE_PX / 2;
  const cy = py + TILE_PX / 2;
  ctx.beginPath();
  switch (token) {
    case 'gold':
      ctx.arc(cx, cy, 5, 0, Math.PI * 2);
      break;
    case 'blade':
      ctx.moveTo(px + 5, py + 15);
      ctx.lineTo(px + 15, py + 5);
      ctx.lineTo(px + 16, py + 7);
      ctx.lineTo(px + 7, py + 16);
      break;
    case 'plate':
      ctx.moveTo(px + 5, py + 4);
      ctx.lineTo(px + 15, py + 4);
      ctx.lineTo(px + 15, py + 11);
      ctx.lineTo(cx, py + 16);
      ctx.lineTo(px + 5, py + 11);
      break;
    case 'potion':
      ctx.rect(px + 8, py + 3, 4, 4);
      ctx.moveTo(px + 6, py + 16);
      ctx.lineTo(px + 14, py + 16);
      ctx.lineTo(px + 12, py + 7);
      ctx.lineTo(px + 8, py + 7);
      break;
    case 'blue_key':
      ctx.arc(px + 7, cy, 3.5, 0, Math.PI * 2);
      ctx.rect(px + 9, cy - 1, 8, 2);
      break;
    case 'gem':
      ctx.moveTo(cx, py + 3);
      ctx.lineTo(px + 15, cy);
      ctx.lineTo(cx, py + 17);
      ctx.lineTo(px + 5, cy);
      break;
    default:
      ctx.rect(px + 6, py + 6, 8, 8);
  }
  ctx.closePath();
  ctx.fill();
  ctx.stroke();
}

function drawGrave(ctx: CanvasRenderingContext2D, px: number, py: number, looted: boolean): void {
  ctx.fillStyle = looted ? C.graveLooted : C.grave;
  ctx.strokeStyle = '#0b1020';
  ctx.beginPath();
  ctx.moveTo(px + 4, py + 17);
  ctx.lineTo(px + 4, py + 8);
  ctx.arc(px + 10, py + 8, 6, Math.PI, 0);
  ctx.lineTo(px + 16, py + 17);
  ctx.closePath();
  ctx.fill();
  ctx.stroke();
  ctx.fillStyle = '#0b1020';
  ctx.fillRect(px + 9, py + 7, 2, 7); // cross
  ctx.fillRect(px + 7, py + 9, 6, 2);
}

export function drawView(ctx: CanvasRenderingContext2D, v: DiveView, flash: boolean): void {
  const w = v.width;
  for (let y = 0; y < v.height; y++) {
    for (let x = 0; x < w; x++) {
      const t = v.tiles[y * w + x] ?? TILE.wall;
      const px = x * TILE_PX;
      const py = y * TILE_PX;
      if (t === TILE.wall) {
        ctx.fillStyle = C.wall;
        ctx.fillRect(px, py, TILE_PX, TILE_PX);
        ctx.fillStyle = C.wallEdge;
        ctx.fillRect(px, py + TILE_PX - 2, TILE_PX, 2);
      } else {
        ctx.fillStyle = (x + y) % 2 === 0 ? C.floor : C.floorAlt;
        ctx.fillRect(px, py, TILE_PX, TILE_PX);
        if (t === TILE.stairs) {
          ctx.fillStyle = C.stairs;
          for (let i = 0; i < 4; i++) ctx.fillRect(px + 2 + i * 2, py + 2 + i * 4, TILE_PX - 4 - i * 4, 3);
        } else if (t !== TILE.floor) {
          // Any other tile kind is a locked door or vault wall.
          ctx.fillStyle = C.door;
          ctx.fillRect(px + 2, py + 1, TILE_PX - 4, TILE_PX - 2);
          ctx.fillStyle = '#ffd166';
          ctx.fillRect(px + TILE_PX - 7, py + TILE_PX / 2 - 1, 3, 3);
        }
      }
    }
  }
  // The way up: an outlined square with an arrow; MOVE.surface works only on this tile.
  {
    const px = v.exit_x * TILE_PX;
    const py = v.exit_y * TILE_PX;
    ctx.strokeStyle = C.exit;
    ctx.lineWidth = 2;
    ctx.strokeRect(px + 1, py + 1, TILE_PX - 2, TILE_PX - 2);
    ctx.fillStyle = C.exit;
    ctx.beginPath();
    ctx.moveTo(px + TILE_PX / 2, py + 3);
    ctx.lineTo(px + TILE_PX - 4, py + TILE_PX / 2);
    ctx.lineTo(px + 4, py + TILE_PX / 2);
    ctx.closePath();
    ctx.fill();
    ctx.fillRect(px + TILE_PX / 2 - 1.5, py + TILE_PX / 2, 3, TILE_PX / 2 - 3);
  }
  for (const g of v.graves) drawGrave(ctx, g.x * TILE_PX, g.y * TILE_PX, g.looted);
  for (const it of v.items) drawItem(ctx, tokenOf(it.item), it.x * TILE_PX, it.y * TILE_PX);
  for (const m of v.monsters) {
    const px = m.x * TILE_PX;
    const py = m.y * TILE_PX;
    ctx.fillStyle = C.monster;
    ctx.strokeStyle = '#0b1020';
    ctx.beginPath();
    ctx.moveTo(px + 10, py + 3);
    ctx.lineTo(px + 17, py + 16);
    ctx.lineTo(px + 3, py + 16);
    ctx.closePath();
    ctx.fill();
    ctx.stroke();
    ctx.fillStyle = '#fff';
    ctx.fillRect(px + 7, py + 11, 2, 2);
    ctx.fillRect(px + 11, py + 11, 2, 2);
  }
  const ppx = v.x * TILE_PX;
  const ppy = v.y * TILE_PX;
  ctx.fillStyle = C.player;
  ctx.strokeStyle = C.playerEdge;
  ctx.lineWidth = 1.5;
  ctx.beginPath();
  ctx.arc(ppx + 10, ppy + 7, 4, 0, Math.PI * 2);
  ctx.fill();
  ctx.stroke();
  ctx.beginPath();
  ctx.roundRect(ppx + 5, ppy + 11, 10, 7, 2);
  ctx.fill();
  ctx.stroke();
  if (flash) {
    ctx.fillStyle = 'rgba(229, 72, 77, 0.28)';
    ctx.fillRect(0, 0, v.width * TILE_PX, v.height * TILE_PX);
  }
}
