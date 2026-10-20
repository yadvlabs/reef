// Shown in the blob view when a file is a Git LFS pointer: what it is and a download link
// to the object behind it (served by the LFS transfer endpoint with the session cookie).
import { Button } from '@primer/react';
import { DownloadIcon, PackageIcon } from '@primer/octicons-react';
import { humanSize, type Repo } from '../../lib/api';
import { parseLfsPointer, type LfsPointerData } from '../../lib/mirror';

export { parseLfsPointer };

export function LfsPointer({ repo, path, pointer }: { repo: Repo; path: string; pointer: LfsPointerData }) {
  const href = `/${repo.full_name}.git/info/lfs/objects/${pointer.oid}`;
  const name = path.split('/').pop() ?? path;
  return (
    <div className="lfs-pointer">
      <PackageIcon size={24} />
      <div className="lfs-pointer-body">
        <strong>Stored with Git LFS</strong>
        <div className="muted small">
          {humanSize(pointer.size)} · sha256:{pointer.oid.slice(0, 12)}… · This file is a pointer; the content lives in the repository's LFS store.
        </div>
        <pre className="cmd lfs-pointer-text">{`version ${pointer.version}\noid sha256:${pointer.oid}\nsize ${pointer.size}`}</pre>
      </div>
      <Button as="a" href={href} leadingVisual={DownloadIcon} size="small" {...({ download: name } as object)}>
        Download
      </Button>
    </div>
  );
}
