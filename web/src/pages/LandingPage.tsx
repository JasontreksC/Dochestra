import { Link } from 'react-router'

export default function LandingPage() {
  return (
    <main className="flex min-h-screen flex-col items-center justify-center gap-6 bg-slate-50 px-6 text-center">
      <h1 className="text-5xl font-bold tracking-tight text-slate-900">Dochestra</h1>
      <p className="max-w-xl text-lg text-slate-600">
        PDF 문서를 중심으로 추출·번역·요약·슬라이드 생성까지 노드를 연결해 구성하는 비주얼 워크플로
        에디터입니다.
      </p>
      <Link
        to="/editor"
        className="rounded-lg bg-indigo-600 px-6 py-3 font-medium text-white hover:bg-indigo-500"
      >
        시작하기
      </Link>
    </main>
  )
}
