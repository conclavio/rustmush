// The cockpit's picture (doc 21 cut G.3c): whether this connection has asked
// the gateway for it, at what look, and the last frame it sent — one PNG,
// kept as the Blob it arrived as and drawn by `PictureView.svelte`.

/** The picture's state for this connection. */
export class PictureStore {
  /** Whether the picture is asked for. */
  on = $state(false);
  /** The look sent with it, millidegrees. */
  lookMdeg = $state(0);
  /** The last frame, or `null` before one arrived or after the picture went
   * off. */
  frame = $state<Blob | null>(null);
  /** Frames received since the picture last went on. */
  received = $state(0);

  /** A binary message from the gateway: the next frame. Ignored while off (a
   * frame already in flight when the picture went off). */
  receive(data: Blob | ArrayBuffer): void {
    if (!this.on) return;
    this.frame = data instanceof ArrayBuffer ? new Blob([data], { type: "image/png" }) : data;
    this.received += 1;
  }

  /** The picture went off, or the connection went: nothing is shown. */
  clear(): void {
    this.frame = null;
    this.received = 0;
  }
}
