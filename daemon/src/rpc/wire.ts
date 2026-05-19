/**
 * NDJSON wire envelope per docs/tui-architecture.md §10.
 * Stub WorldSnapshot here; full shape filled in once Day 5+ wires the daemon-side
 * AgentEventSink and asset/layout modules. Clients are expected to be forward-compatible
 * with extra fields landing in later phases (`schemaVersion: 1` gates breaking changes).
 */

export const PROTO_VERSION = 1;

export type WireMessage = Req | Res | Evt | Hello | HelloAck;

export interface Req {
  kind: 'req';
  reqId: number;
  method: string;
  params: unknown;
}

export type Res =
  | { kind: 'res'; reqId: number; ok: true; data: unknown }
  | { kind: 'res'; reqId: number; ok: false; error: WireError };

export interface WireError {
  code: string;
  message: string;
}

export interface Evt {
  kind: 'evt';
  topic: string;
  seq: number;
  ts: number;
  data: unknown;
}

export interface ClientCapabilities {
  rendering: 'kitty-k' | 'kitty-o' | 'iterm2' | 'sixel' | 'truecolor' | '256' | '16' | 'braille';
  cols: number;
  rows: number;
  cellPx: { w: number; h: number };
  bracketedPaste: boolean;
  mouse: boolean;
}

export interface Hello {
  kind: 'hello';
  token: string;
  clientVersion: string;
  protoVersion: number;
  capabilities: ClientCapabilities;
}

export interface HelloAck {
  kind: 'helloAck';
  daemonVersion: string;
  protoVersion: number;
  bootId: string;
  sessionId: string;
  subscriptions: string[];
  /**
   * Initial world model delivered inline so the client can begin rendering
   * before any `world.snapshot` event lands. Stub until Day 5+ wires real
   * layout/asset/agent state.
   */
  world: WorldSnapshot;
}

/**
 * Stub WorldSnapshot. The real `layout` shape is `OfficeLayout` from
 * `webview-ui/src/office/types.ts`; the daemon doesn't depend on that type
 * directly, so the layout slot is left as a loose record.
 */
export interface WorldSnapshot {
  schemaVersion: 1;
  worldSeed: number;
  layout: Record<string, unknown> | null;
  assets: {
    /** Flat furniture catalog — clients build rotation/state groups client-side. */
    catalog: FurnitureCatalogEntry[];
    /** Counts only; actual pixel data is fetched via assets.requestBlob. */
    characterCount: number;
    floorCount: number;
    wallCount: number;
  };
  agents: [];
}

/** Wire-safe subset of FurnitureAsset — matches shared/assets/manifestUtils FurnitureAsset. */
export interface FurnitureCatalogEntry {
  id: string;
  name: string;
  label: string;
  category: string;
  file: string;
  width: number;
  height: number;
  footprintW: number;
  footprintH: number;
  isDesk: boolean;
  canPlaceOnWalls: boolean;
  canPlaceOnSurfaces?: boolean;
  backgroundTiles?: number;
  groupId?: string;
  orientation?: string;
  state?: string;
  mirrorSide?: boolean;
  rotationScheme?: string;
  animationGroup?: string;
  frame?: number;
}

/** Type guard: minimal validation that `msg` is shaped like a Hello. */
export function isHello(msg: unknown): msg is Hello {
  if (typeof msg !== 'object' || msg === null) return false;
  const m = msg as Partial<Hello>;
  return (
    m.kind === 'hello' &&
    typeof m.token === 'string' &&
    typeof m.clientVersion === 'string' &&
    typeof m.protoVersion === 'number'
  );
}

export function isReq(msg: unknown): msg is Req {
  if (typeof msg !== 'object' || msg === null) return false;
  const m = msg as Partial<Req>;
  return m.kind === 'req' && typeof m.reqId === 'number' && typeof m.method === 'string';
}
