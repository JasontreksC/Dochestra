import { Navigate, Route, Routes } from 'react-router'
import EditorPage from './pages/EditorPage.tsx'
import LandingPage from './pages/LandingPage.tsx'

export default function App() {
  return (
    <Routes>
      <Route path="/" element={<LandingPage />} />
      <Route path="/editor" element={<EditorPage />} />
      <Route path="/editor/:workflowId" element={<EditorPage />} />
      <Route path="*" element={<Navigate to="/" replace />} />
    </Routes>
  )
}
