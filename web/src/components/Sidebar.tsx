import { useState, type FormEvent } from 'react'
import { NavLink, useMatch, useNavigate } from 'react-router'
import { ApiError } from '@/api/client.ts'
import { useCreateWorkflow, useDeleteWorkflow, useWorkflows } from '@/api/queries.ts'

function errorMessage(e: unknown): string {
  return e instanceof ApiError || e instanceof Error ? e.message : '알 수 없는 오류'
}

export default function Sidebar() {
  const navigate = useNavigate()
  const current = useMatch('/editor/:workflowId')?.params.workflowId
  const workflows = useWorkflows()
  const create = useCreateWorkflow()
  const remove = useDeleteWorkflow()
  const [name, setName] = useState('')

  function onSubmit(e: FormEvent) {
    e.preventDefault()
    const trimmed = name.trim()
    if (!trimmed) return
    create.mutate(
      { name: trimmed },
      {
        onSuccess: (wf) => {
          setName('')
          void navigate(`/editor/${wf.id}`)
        },
      },
    )
  }

  function onDelete(id: string, wfName: string) {
    if (!window.confirm(`'${wfName}' 워크플로를 삭제할까요?`)) return
    remove.mutate(id, {
      onSuccess: () => {
        if (current === id) void navigate('/editor')
      },
    })
  }

  return (
    <aside className="flex w-64 shrink-0 flex-col border-r border-slate-200 bg-white">
      <h2 className="px-4 pt-4 text-sm font-semibold text-slate-700">워크플로</h2>

      <form onSubmit={onSubmit} className="flex gap-2 p-4">
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="새 워크플로 이름"
          aria-label="새 워크플로 이름"
          className="min-w-0 flex-1 rounded border border-slate-300 px-2 py-1 text-sm"
        />
        <button
          type="submit"
          disabled={!name.trim() || create.isPending}
          className="rounded bg-indigo-600 px-3 py-1 text-sm text-white disabled:opacity-50"
        >
          생성
        </button>
      </form>
      {create.isError && (
        <p role="alert" className="px-4 pb-2 text-xs text-red-600">
          {errorMessage(create.error)}
        </p>
      )}
      {remove.isError && (
        <p role="alert" className="px-4 pb-2 text-xs text-red-600">
          {errorMessage(remove.error)}
        </p>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-4">
        {workflows.isPending ? (
          <p className="px-2 text-sm text-slate-500">불러오는 중…</p>
        ) : workflows.isError ? (
          <div className="px-2 text-sm">
            <p role="alert" className="text-red-600">
              {errorMessage(workflows.error)}
            </p>
            <button
              type="button"
              onClick={() => void workflows.refetch()}
              className="mt-2 rounded border border-slate-300 px-2 py-1"
            >
              다시 시도
            </button>
          </div>
        ) : workflows.data.length === 0 ? (
          <p className="px-2 text-sm text-slate-500">워크플로가 없습니다</p>
        ) : (
          <ul className="space-y-1">
            {workflows.data.map((wf) => (
              <li key={wf.id} className="group flex items-center">
                <NavLink
                  to={`/editor/${wf.id}`}
                  className={({ isActive }) =>
                    `min-w-0 flex-1 truncate rounded px-2 py-1 text-sm ${
                      isActive
                        ? 'bg-indigo-50 font-medium text-indigo-700'
                        : 'text-slate-700 hover:bg-slate-100'
                    }`
                  }
                >
                  {wf.name}
                </NavLink>
                <button
                  type="button"
                  aria-label={`${wf.name} 삭제`}
                  onClick={() => onDelete(wf.id, wf.name)}
                  className="ml-1 rounded px-2 py-1 text-sm text-slate-400 hover:bg-red-50 hover:text-red-600"
                >
                  ✕
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </aside>
  )
}
