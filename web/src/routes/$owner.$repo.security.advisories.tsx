import { Blankslate } from '@primer/react/experimental';
import { AlertIcon } from '@primer/octicons-react';
import { createFileRoute } from '@tanstack/react-router';

export const Route = createFileRoute('/$owner/$repo/security/advisories')({ component: Advisories });

function Advisories() {
  return (
    <>
      <div className="security-head">
        <h2>Security advisories</h2>
      </div>
      <Blankslate border spacious>
        <Blankslate.Visual>
          <AlertIcon size="medium" />
        </Blankslate.Visual>
        <Blankslate.Heading>No published security advisories</Blankslate.Heading>
        <Blankslate.Description>Advisories let maintainers privately discuss, fix and publish information about security vulnerabilities. Drafting advisories is not available on this instance yet.</Blankslate.Description>
      </Blankslate>
    </>
  );
}
