import { describe, expect, it } from 'vitest'
import { edgeStrokeWidth, formatDataType, SCALAR_COLORS, socketColor } from './socketColors.ts'

describe('socketColors', () => {
  it('matches the CONTEXT §3.2 palette', () => {
    expect(SCALAR_COLORS).toEqual({
      PDF: '#E5484D',
      MD: '#3E63DD',
      JSON: '#F5A524',
      CSV: '#30A46C',
      JPG: '#8E4EC6',
      MP3: '#12A594',
    })
  })

  it('list types share the scalar color', () => {
    expect(socketColor({ scalar: 'JPG', list: true })).toBe(socketColor({ scalar: 'JPG', list: false }))
  })

  it('uses a thicker stroke for lists', () => {
    expect(edgeStrokeWidth({ scalar: 'PDF', list: false })).toBe(2)
    expect(edgeStrokeWidth({ scalar: 'PDF', list: true })).toBe(3)
  })

  it('formats data types', () => {
    expect(formatDataType({ scalar: 'JPG', list: false })).toBe('JPG')
    expect(formatDataType({ scalar: 'JPG', list: true })).toBe('LIST[JPG]')
  })
})
