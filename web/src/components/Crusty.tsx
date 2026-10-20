// Crusty, Reef's mascot, in the moods drawn under web/public/crusty:
// builder (hard hat and blueprints: something to build), front (a happy
// wave), standing, peekaboo (hiding behind the claws: nothing to see here),
// resting (lying on the sand: taking a break), shell-top (tucked in). Served
// as files so that the art changes without a build.
export type CrustyMood = 'builder' | 'front' | 'standing' | 'peekaboo' | 'resting' | 'shell-top';

const FILE: Record<CrustyMood, string> = {
  builder: 'crusty-builder.svg',
  front: 'crusty-front.svg',
  standing: 'crusty-standing.svg',
  peekaboo: 'peekaboo.svg',
  resting: 'resting.svg',
  'shell-top': 'shell-top.svg',
};

export function Crusty({ mood, size = 160, className }: { mood: CrustyMood; size?: number; className?: string }) {
  return <img src={`/crusty/${FILE[mood]}`} alt="" width={size} height={size} className={className ? `crusty-art ${className}` : 'crusty-art'} draggable={false} />;
}

/** A centred empty state: Crusty, a heading, a line of explanation, and whatever follows (a button, a hint). */
export function CrustyEmpty({ mood = 'builder', size = 112, title, children, action }: { mood?: CrustyMood; size?: number; title: React.ReactNode; children?: React.ReactNode; action?: React.ReactNode }) {
  return (
    <div className="crusty-empty">
      <Crusty mood={mood} size={size} />
      <div className="crusty-empty-title">{title}</div>
      {children ? <div className="muted crusty-empty-text">{children}</div> : null}
      {action ? <div className="crusty-empty-action">{action}</div> : null}
    </div>
  );
}
