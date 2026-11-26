import { useRef, useState, type DragEvent, type FormEvent } from 'react'
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
        {bundle.data.files.length === 0 && (
          <li className="text-slate-500">
            아직 업로드한 PDF 가 없습니다 — 위 버튼을 누르거나 PDF 를 끌어다 놓으세요
          </li>
        )}
      </ul>
      {remove.isError && (
        <p role="alert" className="text-xs text-red-600">
          {msg(remove.error)}
        </p>
      )}
    </div>
  )
}

const isPdf = (f: File) => f.type === 'application/pdf' || /\.pdf$/i.test(f.name)

function bundleNameFor(files: File[]): string {
  const base = (files[0]?.name ?? 'bundle').replace(/\.pdf$/i, '')
  const name = files.length > 1 ? `${base} 외 ${files.length - 1}개` : base
  return name.slice(0, 100)
}

export default function BundlePicker({ value, onChange }: Props) {
  const bundles = useBundles()
  const create = useCreateBundle()
  const upload = useUploadBundleFiles()
  const [name, setName] = useState('')
  const [uploading, setUploading] = useState<number | null>(null)
  const [uploadError, setUploadError] = useState<string | null>(null)
  const [dragging, setDragging] = useState(false)
  const inputRef = useRef<HTMLInputElement>(null)

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

  /** Uploads PDFs to the selected bundle, creating (and selecting) one first when none is. */
  async function uploadFiles(candidates: File[]) {
    setUploadError(null)
    const files = candidates.filter(isPdf)
    if (files.length === 0) {
      setUploadError('PDF 파일만 올릴 수 있습니다')
      return
    }
    setUploading(files.length)
    try {
      let bundleId = value
      if (bundleId === null) {
        const created = await create.mutateAsync({ name: bundleNameFor(files) })
        bundleId = created.id
        onChange(created.id)
      }
      await upload.mutateAsync({ bundleId, files })
    } catch (e) {
      setUploadError(msg(e))
    } finally {
      setUploading(null)
    }
  }

  function onDrop(e: DragEvent<HTMLDivElement>) {
    e.preventDefault()
    setDragging(false)
    if (uploading !== null) return
    void uploadFiles([...e.dataTransfer.files])
  }

  const missing = value !== null && bundles.isSuccess && !bundles.data.some((b) => b.id === value)

  return (
    <div className="space-y-3">
      <div
        data-testid="bundle-dropzone"
        onDragOver={(e) => {
          e.preventDefault()
          setDragging(true)
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={onDrop}
        className={`space-y-2 rounded border-2 border-dashed p-3 ${
          dragging ? 'border-blue-500 bg-blue-50' : 'border-slate-300'
        }`}
      >
        <button
          type="button"
          disabled={uploading !== null}
          onClick={() => inputRef.current?.click()}
          className="w-full rounded bg-blue-600 px-3 py-2 text-sm font-medium text-white hover:bg-blue-500 disabled:opacity-50"
        >
          PDF 업로드
        </button>
        <input
          ref={inputRef}
          type="file"
          accept="application/pdf,.pdf"
          multiple
          hidden
          aria-label="PDF 파일 선택"
          onChange={(e) => {
            const files = [...(e.target.files ?? [])]
            e.target.value = ''
            if (files.length > 0) void uploadFiles(files)
          }}
        />
        <p className="text-center text-xs text-slate-500">또는 PDF 를 여기로 끌어다 놓으세요</p>
        {uploading !== null && (
          <p className="text-xs text-slate-600">업로드 중… ({uploading}개)</p>
        )}
        {uploadError && (
          <p role="alert" className="text-xs text-red-600">
            {uploadError}
          </p>
        )}
        {value !== null && !missing && <BundleFiles key={value} bundleId={value} />}
      </div>

      <section className="space-y-2 border-t border-slate-200 pt-3">
        <h4 className="text-xs font-semibold text-slate-500">다른 번들 선택</h4>
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
        {create.isError && !uploadError && (
          <p role="alert" className="text-xs text-red-600">
            {msg(create.error)}
          </p>
        )}
      </section>
    </div>
  )
}
