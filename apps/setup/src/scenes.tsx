import { Nyu, type NyuMood } from '@uwusuite/design';

// The installer's pictures: Nyu from @uwusuite/design in UwUSweep's box, one mood per moment.
// Decorative only; the sentence beside the picture is the message.

function Scene({ mood, className }: { mood: NyuMood; className?: string }) {
  return (
    <div className={`flex w-full justify-center ${className ?? ''}`} aria-hidden>
      <Nyu shell="box" mood={mood} size={160} title="" />
    </div>
  );
}

export const WelcomeScene = () => <Scene mood="happy" />;
export const WorkingScene = () => <Scene mood="sparkle" className="setup-hop" />;
export const DoneScene = () => <Scene mood="cheer" className="setup-pop" />;
export const ErrorScene = () => <Scene mood="sad" />;
export const PuzzledScene = () => <Scene mood="puzzled" />;
export const GoodbyeScene = () => <Scene mood="sleepy" />;
