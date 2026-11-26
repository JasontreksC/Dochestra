import { create } from 'zustand'
import type { RagSource } from '@/bindings/RagSource'

export type ChatMessage =
  | { id: number; role: 'user'; text: string }
  | { id: number; role: 'assistant'; answer: string; sources: RagSource[] }
  | { id: number; role: 'error'; message: string }

type NewMessage =
  | { role: 'user'; text: string }
  | { role: 'assistant'; answer: string; sources: RagSource[] }
  | { role: 'error'; message: string }

interface RagChatState {
  /** Conversation per RAG index id; lives for the browser session (not persisted). */
  histories: Readonly<Record<string, readonly ChatMessage[]>>
  append: (indexId: string, message: NewMessage) => void
  clear: (indexId: string) => void
  clearAll: () => void
}

let nextId = 1
const NO_MESSAGES: readonly ChatMessage[] = []

export const useRagChatStore = create<RagChatState>()((set) => ({
  histories: {},
  append: (indexId, message) =>
    set((s) => ({
      histories: {
        ...s.histories,
        [indexId]: [...(s.histories[indexId] ?? []), { ...message, id: nextId++ } as ChatMessage],
      },
    })),
  clear: (indexId) =>
    set((s) => {
      const rest = { ...s.histories }
      delete rest[indexId]
      return { histories: rest }
    }),
  clearAll: () => set({ histories: {} }),
}))

export const useRagHistory = (indexId: string): readonly ChatMessage[] =>
  useRagChatStore((s) => s.histories[indexId] ?? NO_MESSAGES)
