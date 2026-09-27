const UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

/** Formats a size in bytes as a readable string ("1.2 MB"). */
export function formatBytes(bytes: number): string {
  if (bytes <= 0) {
    return "0 B";
  }

  const exponent = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), UNITS.length - 1);
  const value = bytes / 1024 ** exponent;
  const unit = UNITS[exponent] ?? "B";

  return `${exponent === 0 ? value.toFixed(0) : value.toFixed(1)} ${unit}`;
}

/** Truncates a long identifier or checksum, keeping both ends. */
export function truncateMiddle(value: string, visibleChars = 8): string {
  if (value.length <= visibleChars * 2 + 3) {
    return value;
  }
  return `${value.slice(0, visibleChars)}…${value.slice(-visibleChars)}`;
}
