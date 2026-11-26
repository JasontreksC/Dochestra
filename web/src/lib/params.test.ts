import { describe, expect, it } from 'vitest'
import { isUuid, validateParams } from './params.ts'

const UUID = '3f2b8c1e-5d4a-4b7e-9c10-0a1b2c3d4e5f'

describe('validateParams (mirrors engine h_param_rules)', () => {
  it('requires an object', () => {
    for (const p of [[], null, 'x', 5, undefined]) {
      expect(validateParams('extract', p)).toBe('params 는 JSON 객체여야 합니다')
    }
    expect(validateParams('extract', {})).toBeNull()
  })

  it('bundle: bundleId must be a UUID string', () => {
    for (const p of [{}, { bundleId: null }, { bundleId: 'abc' }, { bundleId: 5 }]) {
      expect(validateParams('bundle', p)).toBe('bundleId 가 올바른 UUID 가 아닙니다')
    }
    expect(validateParams('bundle', { bundleId: UUID })).toBeNull()
  })

  it('pick_pdf / pick_jpg: index must be a non-negative integer', () => {
    for (const kind of ['pick_pdf', 'pick_jpg'] as const) {
      for (const p of [{ index: -1 }, { index: 'a' }, { index: 1.5 }, {}]) {
        expect(validateParams(kind, p)).toBe('index 는 0 이상의 정수여야 합니다')
      }
      for (const p of [{ index: 0 }, { index: 7 }]) expect(validateParams(kind, p)).toBeNull()
    }
  })

  it('json_input: value is required and non-null', () => {
    expect(validateParams('json_input', {})).toBe('value 가 필요합니다')
    expect(validateParams('json_input', { value: null })).toBe('value 가 필요합니다')
    expect(validateParams('json_input', { value: {} })).toBeNull()
    expect(validateParams('json_input', { value: 0 })).toBeNull()
    expect(validateParams('json_input', { value: false })).toBeNull()
  })

  it('other kinds only need an object', () => {
    expect(validateParams('translate', { anything: 1 })).toBeNull()
  })
})

describe('isUuid', () => {
  it('accepts the forms the engine accepts', () => {
    expect(isUuid(UUID)).toBe(true)
    expect(isUuid(UUID.replaceAll('-', ''))).toBe(true)
    expect(isUuid(`{${UUID}}`)).toBe(true)
    expect(isUuid(`urn:uuid:${UUID}`)).toBe(true)
    expect(isUuid(UUID.toUpperCase())).toBe(true)
  })
  it('rejects malformed values', () => {
    for (const v of ['', 'abc', UUID.slice(1), `${UUID}0`, UUID.replace('3f', 'zz')]) {
      expect(isUuid(v)).toBe(false)
    }
  })
})
