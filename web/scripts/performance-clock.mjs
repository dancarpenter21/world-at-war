// Anchor timestamps once, then advance with the monotonic clock. WSL wall-clock
// corrections must not change deadlines, stream gaps, or measurement windows.
export function createClock(wall = Date.now, monotonic = () => performance.now()) {
  const epoch = wall(), origin = monotonic();
  return { epoch, now: () => epoch + monotonic() - origin };
}
