import { useEffect, useId, useRef, useState, type KeyboardEvent } from 'react'
import type { RagSource } from '@/bindings/RagSource'
import { ApiError } from '@/api/client.ts'
import { useQueryRag } from '@/api/queries.ts'
import { splitCitations } from '@/lib/citations.ts'
import { useRagIndex } from '@/lib/ragIndex.ts'
import { useEditorStore } from '@/store/editorStore.ts'
import { useRagChatStore, useRagHistory, type ChatMessage } from '@/store/ragChatStore.ts'

const QUESTION_MAX = 2000
const PREVIEW_CHARS = 300
const TOP_K_OPTIONS = Array.from({ length: 20 }, (_, i) => i + 1)

function SourceItem({
  domId,
  n,
  source,
  expanded,
  highlighted,
  onToggle,
}: {
  domId: string
  n: number
  source: RagSource
  expanded: boolean
  highlighted: boolean
  onToggle: () => void
}) {
  const long = source.content.length > PREVIEW_CHARS
  const text = expanded || !long ? source.content : `${source.content.slice(0, PREVIEW_CHARS)}…`
  return (
    <li
      id={domId}
      data-testid="rag-source"
      className={`rounded border p-1.5 ${highlighted ? 'border-indigo-400 bg-indigo-50' : 'border-slate-200 bg-white'}`}
    >
      <p className="text-[10px] text-slate-500">
        <span className="font-semibold text-slate-700">[{n}]</span> 청크 #{source.chunkIndex} · 점수{' '}
        {source.score.toFixed(2)}
      </p>
      <p className="mt-0.5 text-[11px] break-words whitespace-pre-wrap text-slate-700">{text}</p>
      {long && (
        <button
          type="button"
          onClick={onToggle}
          aria-expanded={expanded}
          className="mt-0.5 text-[10px] text-indigo-600 underline"
        >
          {expanded ? '접기' : '전체 보기'}
        </button>
      )}
    </li>
  )
}

function AnswerBubble({ answer, sources }: { answer: string; sources: RagSource[] }) {
  const prefix = useId()
  const [open, setOpen] = useState(false)
  const [expanded, setExpanded] = useState<ReadonlySet<number>>(new Set())
  const [focus, setFocus] = useState<{ n: number; seq: number } | null>(null)
  const domId = (n: number) => `${prefix}-src-${n}`

  useEffect(() => {
    if (!focus) return
    document
      .getElementById(`${prefix}-src-${focus.n}`)
      ?.scrollIntoView?.({ block: 'nearest', behavior: 'smooth' })
  }, [focus, prefix])

  const toggle = (n: number) =>
    setExpanded((prev) => {
      const next = new Set(prev)
      if (next.has(n)) next.delete(n)
      else next.add(n)
      return next
    })

  const cite = (n: number) => {
    setOpen(true)
    setExpanded((prev) => new Set(prev).add(n))
    setFocus((f) => ({ n, seq: (f?.seq ?? 0) + 1 }))
  }

  return (
    <div className="max-w-[90%] self-start rounded-lg bg-slate-100 px-2.5 py-1.5 text-xs text-slate-800">
      <p data-testid="rag-answer" className="break-words whitespace-pre-wrap">
        {splitCitations(answer, sources.length).map((part, i) =>
          part.kind === 'text' ? (
            <span key={i}>{part.text}</span>
          ) : (
            <button
              key={i}
              type="button"
              aria-label={`근거 ${part.n}`}
              onClick={() => cite(part.n)}
              className="mx-0.5 inline-flex h-4 min-w-4 items-center justify-center rounded bg-indigo-100 px-1 align-text-top text-[10px] font-semibold text-indigo-700 hover:bg-indigo-200"
            >
              {part.n}
            </button>
          ),
        )}
      </p>
      {sources.length > 0 && (
        <div className="mt-1.5">
          <button
            type="button"
            onClick={() => setOpen((o) => !o)}
            aria-expanded={open}
            className="text-[11px] text-slate-600 underline"
          >
            근거 {sources.length}개
          </button>
          {open && (
            <ol className="mt-1 space-y-1">
              {sources.map((s, i) => (
                <SourceItem
                  key={i}
                  domId={domId(i + 1)}
                  n={i + 1}
                  source={s}
                  expanded={expanded.has(i + 1)}
                  highlighted={focus?.n === i + 1}
                  onToggle={() => toggle(i + 1)}
                />
              ))}
            </ol>
          )}
        </div>
      )}
    </div>
  )
}

function Message({ message }: { message: ChatMessage }) {
  switch (message.role) {
    case 'user':
      return (
        <p
          data-testid="rag-question"
          className="max-w-[90%] self-end rounded-lg bg-indigo-600 px-2.5 py-1.5 text-xs break-words whitespace-pre-wrap text-white"
        >
          {message.text}
        </p>
      )
    case 'assistant':
      return <AnswerBubble answer={message.answer} sources={message.sources} />
    case 'error':
      return (
        <p
          role="alert"
          className="max-w-[90%] self-start rounded-lg border border-red-200 bg-red-50 px-2.5 py-1.5 text-xs break-words text-red-700"
        >
          {message.message}
        </p>
      )
  }
}

const errorText = (e: unknown): string =>
  e instanceof ApiError
    ? `오류 (${e.status} ${e.code}): ${e.message}`
    : `오류: ${e instanceof Error ? e.message : '알 수 없는 오류'}`

function ChatBox({ indexId }: { indexId: string }) {
  const history = useRagHistory(indexId)
  const append = useRagChatStore((s) => s.append)
  const clear = useRagChatStore((s) => s.clear)
  const query = useQueryRag()
  const [input, setInput] = useState('')
  const [topK, setTopK] = useState(5)
  const listRef = useRef<HTMLDivElement>(null)

  const question = input.trim()
  const valid = question.length >= 1 && question.length <= QUESTION_MAX
  const pending = query.isPending

  useEffect(() => {
    const el = listRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [history.length, pending])

  async function send() {
    if (!valid || pending) return
    append(indexId, { role: 'user', text: question })
    try {
      const res = await query.mutateAsync({ indexId, question, topK })
      append(indexId, { role: 'assistant', answer: res.answer, sources: res.sources })
      setInput('')
    } catch (e) {
      append(indexId, { role: 'error', message: errorText(e) })
    }
  }

  function onKeyDown(e: KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key !== 'Enter' || e.shiftKey || e.nativeEvent.isComposing) return
    e.preventDefault()
    void send()
  }

  return (
    <div className="flex h-[28rem] min-h-[320px] flex-col rounded border border-slate-200">
      <div
        ref={listRef}
        aria-label="대화"
        role="log"
        className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto p-2"
      >
        {history.length === 0 && !pending && (
          <p className="text-xs text-slate-400">문서에 대해 질문해 보세요</p>
        )}
        {history.map((m) => (
          <Message key={m.id} message={m} />
        ))}
        {pending && (
          <p role="status" className="self-start text-xs text-slate-500">
            답변 생성 중…
          </p>
        )}
      </div>
      <div className="space-y-1.5 border-t border-slate-200 p-2">
        <textarea
          aria-label="질문 입력"
          value={input}
          rows={2}
          maxLength={QUESTION_MAX}
          placeholder="질문 (Enter 전송, Shift+Enter 줄바꿈)"
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={onKeyDown}
          className="block w-full resize-none rounded border border-slate-300 px-2 py-1 text-xs"
        />
        <div className="flex items-center gap-2">
          <label className="flex items-center gap-1 text-[11px] text-slate-600">
            topK
            <select
              aria-label="topK"
              value={topK}
              onChange={(e) => setTopK(Number(e.target.value))}
              className="rounded border border-slate-300 px-1 py-0.5 text-[11px]"
            >
              {TOP_K_OPTIONS.map((k) => (
                <option key={k} value={k}>
                  {k}
                </option>
              ))}
            </select>
          </label>
          <button
            type="button"
            onClick={() => clear(indexId)}
            disabled={history.length === 0 || pending}
            className="text-[11px] text-slate-500 underline disabled:opacity-40"
          >
            대화 지우기
          </button>
          <button
            type="button"
            onClick={() => void send()}
            disabled={!valid || pending}
            className="ml-auto rounded bg-indigo-600 px-3 py-1 text-xs text-white disabled:opacity-50"
          >
            보내기
          </button>
        </div>
      </div>
    </div>
  )
}

/** Chat against the RAG index built by rag node `nodeId` (Inspector, kind `rag`). */
export default function RagChat({
  nodeId,
  selectedRunId,
}: {
  nodeId: string
  selectedRunId: string | null
}) {
  const workflowId = useEditorStore((s) => s.workflowId)
  const index = useRagIndex(workflowId, nodeId, selectedRunId)

  return (
    <section aria-label="RAG 챗봇" className="space-y-2">
      <h3 className="text-xs font-semibold text-slate-500">RAG 챗봇</h3>
      {index.status === 'loading' ? (
        <p className="text-xs text-slate-500">인덱스를 찾는 중…</p>
      ) : index.status === 'error' ? (
        <p role="alert" className="text-xs text-red-600">
          {index.message}
        </p>
      ) : index.status === 'none' ? (
        <div className="space-y-1 text-xs text-slate-600">
          <p>워크플로를 실행해 RAG 인덱스를 만든 뒤 질문할 수 있습니다</p>
          <p className="text-slate-400">이 RAG 챗봇 노드가 실행에서 ‘완료’ 상태가 되어야 합니다</p>
        </div>
      ) : (
        <>
          <p className="text-[11px] text-slate-500" title={index.embeddingModel}>
            인덱스: 청크 {index.chunkCount}개 · 실행 {index.runId.slice(0, 8)}
          </p>
          <ChatBox key={index.indexId} indexId={index.indexId} />
        </>
      )}
    </section>
  )
}
