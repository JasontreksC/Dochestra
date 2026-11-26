import { afterEach, describe, expect, it, vi } from 'vitest'
import { newId } from './ids.ts'

const UUID_V4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/

/** Simulates an insecure context (plain http on a non-localhost host): no `crypto.randomUUID`. */
function stubInsecureCrypto() {
  const real = globalThis.crypto
  vi.stubGlobal('crypto', {
    getRandomValues: real.getRandomValues.bind(real),
  })
}

afterEach(() => vi.unstubAllGlobals())

describe('newId', () => {
  it('uses crypto.randomUUID when available', () => {
    expect(newId()).toMatch(UUID_V4)
  })

  it('falls back to getRandomValues without crypto.randomUUID', () => {
    stubInsecureCrypto()
    expect(typeof crypto.randomUUID).toBe('undefined')
    const ids = new Set<string>()
    for (let i = 0; i < 1000; i++) {
      const id = newId()
      expect(id).toMatch(UUID_V4)
      ids.add(id)
    }
    expect(ids.size).toBe(1000)
  })
})
