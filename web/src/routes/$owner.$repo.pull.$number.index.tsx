import { createFileRoute } from '@tanstack/react-router';
import { PullConversation } from '../components/pulls/Conversation';
import { usePull } from './$owner.$repo.pull.$number';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/pull/$number/')({ component: Page });

function Page() {
  const repo = useRepo();
  const pr = usePull();
  return <PullConversation repo={repo} pr={pr} />;
}
