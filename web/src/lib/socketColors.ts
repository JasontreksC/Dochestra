import type { DataType } from '@/bindings/DataType'
import type { ScalarType } from '@/bindings/ScalarType'

/** CONTEXT §3.2 */
export const SCALAR_COLORS: Record<ScalarType, string> = {
  PDF: '#E5484D',
  MD: '#3E63DD',
  JSON: '#F5A524',
  CSV: '#30A46C',
  JPG: '#8E4EC6',
  MP3: '#12A594',
}

export const socketColor = (dt: DataType): string => SCALAR_COLORS[dt.scalar]

export const edgeStrokeWidth = (dt: DataType): number => (dt.list ? 3 : 2)

export const formatDataType = (dt: DataType): string =>
  dt.list ? `LIST[${dt.scalar}]` : dt.scalar
