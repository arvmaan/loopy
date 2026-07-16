import { Routes, Route, NavLink, Outlet } from 'react-router-dom'
import { ProjectListV2 } from './pages/ProjectListV2'
import { PipelineView } from './pages/PipelineView'

function V2Layout() {
  return (
    <div className="min-h-screen bg-neutral-950 text-neutral-200 font-mono text-base">
      <header className="px-6 py-3 border-b border-neutral-800">
        <NavLink to="/" className="text-lg font-semibold text-neutral-100 tracking-wide">Loopy</NavLink>
      </header>
      <main className="flex-1 overflow-auto">
        <Outlet />
      </main>
    </div>
  )
}

export function App() {
  return (
    <Routes>
      <Route element={<V2Layout />}>
        <Route index element={<ProjectListV2 />} />
        <Route path="projects/:name" element={<PipelineView />} />
      </Route>
    </Routes>
  )
}
