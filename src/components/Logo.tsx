import appIcon from '../assets/app-icon.png'

/** The approved Relay artwork, shared with the desktop and website icons. */
export function Logo({ size = 24 }: { size?: number }) {
  return <img src={appIcon} width={size} height={size} alt="" aria-hidden draggable={false} />
}
