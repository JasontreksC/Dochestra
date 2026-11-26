import type { NodeKind } from '@/bindings/NodeKind'

const HEX = '[0-9a-fA-F]'
// Same forms the engine's Uuid::parse_str accepts: hyphenated, simple, braced, urn.
const UUID_BODY = new RegExp(
  `^(?:${HEX}{8}-${HEX}{4}-${HEX}{4}-${HEX}{4}-${HEX}{12}|${HEX}{32})$`,
)

export function isUuid(value: string): boolean {
  let s = value
  if (s.toLowerCase().startsWith('urn:uuid:')) s = s.slice('urn:uuid:'.length)
  else if (s.startsWith('{') && s.endsWith('}')) s = s.slice(1, -1)
  return UUID_BODY.test(s)
}

/**
 * Mirrors engine/src/graph/validator.rs `validate_params` (rules and messages).
 * Returns an error message, or null when the params are valid.
 */
export function validateParams(kind: NodeKind, params: unknown): string | null {
  if (typeof params !== 'object' || params === null || Array.isArray(params)) {
    return 'params 는 JSON 객체여야 합니다'
  }
  const p = params as Record<string, unknown>
  switch (kind) {
    case 'bundle':
      return typeof p.bundleId === 'string' && isUuid(p.bundleId)
        ? null
        : 'bundleId 가 올바른 UUID 가 아닙니다'
    case 'pick_pdf':
    case 'pick_jpg':
      return typeof p.index === 'number' && Number.isInteger(p.index) && p.index >= 0
        ? null
        : 'index 는 0 이상의 정수여야 합니다'
    case 'json_input':
      return p.value !== undefined && p.value !== null ? null : 'value 가 필요합니다'
    default:
      return null
  }
}
