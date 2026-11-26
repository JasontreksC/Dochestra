import { useState, type FormEvent } from 'react'
import { ApiError } from '@/api/client.ts'
import {
  useBundle,
  useBundles,
  useCreateBundle,
  useDeleteBundleFile,
  useUploadBundleFiles,
} from '@/api/queries.ts'

interface Props {
  value: string | null
  onChange: (id: string | null) => void
}

function formatSize(bytes: number): string {
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

const msg = (e: unknown) => (e instanceof ApiError || e instanceof Error ? e.message : '오류')

function BundleFiles({ bundleId }: { bundleId: string }) {
  const bundle = useBundle(bundleId)
  const upload = useUploadBundleFiles()
  const remove = useDeleteBundleFile()

  if (bundle.isPending) return <p className="text-xs text-slate-500">불러오는 중…</p>
  if (bundle.isError)
    return (
      <p role="alert" className="text-xs text-red-600">
        {msg(bundle.error)}
      </p>
    )

  return (
    <div className="space-y-2">
      <ul className="space-y-1 text-xs">
        {bundle.data.files.map((f) => (
          <li key={f.id} className="flex items-center justify-between gap-2">
            <span className="min-w-0 flex-1 truncate" title={f.originalName}>
              {f.originalName}
            </span>
            <span className="text-slate-500">{formatSize(f.sizeBytes)}</span>
            <button
              type="button"
              aria-label={`${f.originalName} 삭제`}
              onClick={() => remove.mutate({ bundleId, fileId: f.id })}
              className="text-slate-400 hover:text-red-600"
            >
              ✕
            </button>
          </li>
        ))}
        {bundle.data.files.length === 0 && <li className="text-slate-500">파일이 없습니다</li>}
      </ul>
      <input
        type="file"
        accept="application/pdf,.pdf"
        multiple
        aria-label="PDF 업로드"
        disabled={upload.isPending}
        onChange={(e) => {
          const files = [...(e.target.files ?? [])]
          e.target.value = ''
          if (files.length > 0) upload.mutate({ bundleId, files })
        }}
        className="block w-full text-xs"
      />
      {upload.isPending && <p className="text-xs text-slate-500">업로드 중…</p>}
      {upload.isError && (
        <p role="alert" className="text-xs text-red-600">
          {msg(upload.error)}
        </p>
      )}
      {remove.isError && (
        <p role="alert" className="text-xs text-red-600">
          {msg(remove.error)}
        </p>
      )}
    </div>
  )
}

export default function BundlePicker({ value, onChange }: Props) {
  const bundles = useBundles()
  const create = useCreateBundle()
  const [name, setName] = useState('')

  function onCreate(e: FormEvent) {
    e.preventDefault()
    const trimmed = name.trim()
    if (!trimmed) return
    create.mutate(
      { name: trimmed },
      {
        onSuccess: (b) => {
          setName('')
          onChange(b.id)
        },
      },
    )
  }

  const missing = value !== null && bundles.isSuccess && !bundles.data.some((b) => b.id === value)

  return (
    <div className="space-y-3">
      <label className="block text-xs text-slate-600">
        번들
        <select
          aria-label="번들"
          value={value ?? ''}
          onChange={(e) => onChange(e.target.value || null)}
          className="mt-1 block w-full rounded border border-slate-300 px-2 py-1 text-sm"
        >
          <option value="">선택 없음</option>
          {value !== null && missing && <option value={value}>{value}</option>}
          {bundles.data?.map((b) => (
            <option key={b.id} value={b.id}>{`${b.name} (${b.fileCount})`}</option>
          ))}
        </select>
      </label>
      {bundles.isError && (
        <p role="alert" className="text-xs text-red-600">
          {msg(bundles.error)}
        </p>
      )}
      {missing && (
        <p role="alert" className="text-xs text-amber-600">
          번들을 찾을 수 없습니다
        </p>
      )}

      <form onSubmit={onCreate} className="flex gap-2">
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="새 번들 이름"
          aria-label="새 번들 이름"
          className="min-w-0 flex-1 rounded border border-slate-300 px-2 py-1 text-xs"
        />
        <button
          type="submit"
          disabled={!name.trim() || create.isPending}
          className="rounded bg-slate-700 px-2 py-1 text-xs text-white disabled:opacity-50"
        >
          새 번들
        </button>
      </form>
      {create.isError && (
        <p role="alert" className="text-xs text-red-600">
          {msg(create.error)}
        </p>
      )}

      {value !== null && !missing && <BundleFiles key={value} bundleId={value} />}
    </div>
  )
}
