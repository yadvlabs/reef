// Upload or remove a profile picture for a user or an organization.
import { Avatar, Button } from '@primer/react';
import { UploadIcon, TrashIcon } from '@primer/octicons-react';
import { useMutation } from '@tanstack/react-query';
import { useRef } from 'react';
import { ApiError } from '../lib/api';

export function AvatarUpload({ avatarUrl, endpoint, square, onChanged }: { avatarUrl: string; endpoint: string; square?: boolean; onChanged: () => void }) {
  const input = useRef<HTMLInputElement>(null);
  const upload = useMutation({
    mutationFn: async (file: File) => {
      const res = await fetch(`/api/reef${endpoint}`, { method: 'POST', headers: { 'Content-Type': file.type }, body: file, credentials: 'same-origin' });
      if (!res.ok) throw new ApiError(res.status, (await res.json().catch(() => ({ message: res.statusText }))).message ?? res.statusText, null);
    },
    onSuccess: onChanged,
  });
  const remove = useMutation({
    mutationFn: async () => {
      const res = await fetch(`/api/reef${endpoint}`, { method: 'DELETE', credentials: 'same-origin' });
      if (!res.ok) throw new ApiError(res.status, res.statusText, null);
    },
    onSuccess: onChanged,
  });
  return (
    <div className="avatar-upload">
      <Avatar src={avatarUrl} size={200} square={square} alt="" />
      <div className="form-stack" style={{ gap: 8 }}>
        <input ref={input} type="file" accept="image/png,image/jpeg,image/gif,image/webp" style={{ display: 'none' }} onChange={(e) => e.target.files?.[0] && upload.mutate(e.target.files[0])} />
        <Button size="small" leadingVisual={UploadIcon} onClick={() => input.current?.click()} loading={upload.isPending}>
          Upload a photo…
        </Button>
        <Button size="small" variant="invisible" leadingVisual={TrashIcon} onClick={() => remove.mutate()}>
          Remove photo
        </Button>
        {upload.isError ? <span className="small" style={{ color: 'var(--fgColor-danger)' }}>{(upload.error as Error).message}</span> : null}
        <span className="muted small">PNG, JPEG, GIF or WebP up to 2 MB.</span>
      </div>
    </div>
  );
}
