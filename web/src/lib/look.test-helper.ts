/** Tests' looks (fcc's Look, voxel-grid.ts's cells) of colours, and back; on no other module, so any test may take them. */
/**
 * What a voxel of colour `color` ('#rrggbb', sRGB) and `alpha` looks
 * like (fcc's Look): red in the lowest byte, alpha in the highest.
 */
export function lookOf(color: string, alpha = 255): number {
  const rgb = parseInt(color.slice(1), 16)
  return ((alpha << 24) | ((rgb & 0xff) << 16) | (rgb & 0xff00) | ((rgb >>> 16) & 0xff)) >>> 0
}

/** The colour '#rrggbb' of look `look`. */
export function colorOf(look: number): string {
  const rgb = ((look & 0xff) << 16) | (look & 0xff00) | ((look >>> 16) & 0xff)
  return `#${rgb.toString(16).padStart(6, '0')}`
}
