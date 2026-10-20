// Reef's logo, Crusty in a black disc, served from /logo.svg (web/public, a
// copy of the Reef website's) rather than drawn here: change the file to
// change the brand. ?v= is the build id, so browsers never keep a stale copy.
const LOGO = `/logo.svg?v=${import.meta.env.VITE_BUILD_ID ?? '0'}`;

/** The disc mark at a given size; decorative (the product name is next to it). */
export function Logo({ size = 32 }: { size?: number; color?: string }) {
  return <img src={LOGO} alt="" width={size} height={size} style={{ display: 'block', width: size, height: size }} />;
}

/** The mark where it stands for Reef itself (a landing page, sign-in). */
export function Wordmark({ height = 28 }: { height?: number; color?: string }) {
  return <img src={LOGO} alt="Reef" width={height} height={height} style={{ display: 'block', width: height, height, margin: '0 auto' }} />;
}
