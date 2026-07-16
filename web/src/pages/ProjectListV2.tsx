import { useState, useEffect } from 'react';
import { useNavigate } from 'react-router-dom';
import type { ProjectSummary, Phase } from '../types/v2';

function PhaseTag({ phase }: { phase: Phase }) {
  const colors: Record<Phase, string> = {
    initializing: 'bg-neutral-700 text-neutral-300',
    scanning: 'bg-blue-900/50 text-blue-400',
    planning: 'bg-blue-900/50 text-blue-400',
    awaiting_plan_review: 'bg-yellow-900/50 text-yellow-400',
    setting_up_workspaces: 'bg-blue-900/50 text-blue-400',
    running_tracks: 'bg-blue-900/50 text-blue-400',
    awaiting_code_review: 'bg-yellow-900/50 text-yellow-400',
    awaiting_test_plan: 'bg-yellow-900/50 text-yellow-400',
    test_flying: 'bg-blue-900/50 text-blue-400',
    awaiting_test_review: 'bg-yellow-900/50 text-yellow-400',
    landing: 'bg-blue-900/50 text-blue-400',
    complete: 'bg-green-900/50 text-green-400',
    failed: 'bg-red-900/50 text-red-400',
  };

  const labels: Record<Phase, string> = {
    initializing: 'Starting',
    scanning: 'Scanning',
    planning: 'Planning',
    awaiting_plan_review: 'Review Plan',
    setting_up_workspaces: 'Setup',
    running_tracks: 'Running',
    awaiting_code_review: 'Flight Check',
    awaiting_test_plan: 'Test Plan',
    test_flying: 'Test Flight',
    awaiting_test_review: 'Test Review',
    landing: 'Landing',
    complete: 'Done',
    failed: 'Failed',
  };

  return (
    <span className={`text-xs px-2 py-0.5 rounded ${colors[phase]}`}>
      {labels[phase]}
    </span>
  );
}

function timeAgo(dateStr: string): string {
  const diff = Date.now() - new Date(dateStr).getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return 'just now';
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

interface Block {
  id: string;
  kind: string;
  label: string;
  description?: string;
  optional?: boolean;
  checkpoint?: boolean;
  locked?: boolean;
}

interface BlockKind {
  kind: string;
  label: string;
  description: string;
  category?: string;
}

// Display order for block categories in the library.
const CATEGORY_ORDER = ['Understand', 'Design', 'Build', 'Review & Verify', 'Ship'];

export function ProjectListV2() {
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const [newIdea, setNewIdea] = useState('');
  const [creating, setCreating] = useState(false);
  const [planning, setPlanning] = useState(false);
  // The editable proposed block list (null until the user plans an idea).
  const [blocks, setBlocks] = useState<Block[] | null>(null);
  const [reason, setReason] = useState<string>('');
  const [dragIndex, setDragIndex] = useState<number | null>(null);
  const [palette, setPalette] = useState<BlockKind[]>([]);
  const [showLibrary, setShowLibrary] = useState(false);
  const [sessionName, setSessionName] = useState('');
  const [enriching, setEnriching] = useState(false);
  const [enriched, setEnriched] = useState<string | null>(null); // proposed enriched prompt awaiting approval
  const navigate = useNavigate();

  // Enrich the rough prompt into a better one for the user to approve.
  const enrich = async () => {
    if (!newIdea.trim()) return;
    setEnriching(true);
    setEnriched(null);
    try {
      const resp = await fetch('/api/enrich-prompt', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ prompt: newIdea.trim() }),
      });
      if (!resp.ok) {
        alert('Prompt enrichment is unavailable (backend agent not reachable).');
      } else {
        const d = await resp.json();
        setEnriched(d.enriched ?? null);
      }
    } catch {
      alert('Prompt enrichment failed.');
    }
    setEnriching(false);
  };

  // Load the add-block palette (all block kinds) from the backend.
  useEffect(() => {
    fetch('/api/block-kinds')
      .then(r => r.json())
      .then(d => setPalette(d.kinds ?? []))
      .catch(() => {});
  }, []);

  useEffect(() => {
    const fetchProjects = () => {
      fetch('/api/projects')
        .then(r => r.json())
        .then(setProjects)
        .catch(() => {});
    };
    fetchProjects();
    const interval = setInterval(fetchProjects, 5000);
    return () => clearInterval(interval);
  }, []);

  // Step 1: ask the planner to PROPOSE a block list for the idea (editable next).
  const planIt = async () => {
    if (!newIdea.trim()) return;
    setPlanning(true);
    try {
      const resp = await fetch('/api/plan-blocks', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ prompt: newIdea.trim() }),
      });
      const d = await resp.json();
      setBlocks(d.blocks ?? []);
      setReason(d.reason ?? '');
    } catch {}
    setPlanning(false);
  };

    // Block-list editing. Locked blocks (Scan first, Land last) can't move/remove,
  // and the movable range is bounded between the leading/trailing locked blocks.
  const movableBounds = (bs: Block[]): [number, number] => {
    let lo = 0;
    let hi = bs.length - 1;
    while (lo < bs.length && bs[lo]?.locked) lo++;
    while (hi >= 0 && bs[hi]?.locked) hi--;
    return [lo, hi];
  };

  const move = (i: number, dir: -1 | 1) => {
    setBlocks(bs => {
      if (!bs) return bs;
      if (bs[i]?.locked) return bs;
      const j = i + dir;
      const [lo, hi] = movableBounds(bs);
      if (j < lo || j > hi) return bs; // can't cross a locked bookend
      const next = [...bs];
      const a = next[i]!;
      const b = next[j]!;
      next[i] = b;
      next[j] = a;
      return next;
    });
  };

  const remove = (i: number) =>
    setBlocks(bs => (bs && !bs[i]?.locked ? bs.filter((_, k) => k !== i) : bs));

  const addBlock = (k: BlockKind) => {
    setBlocks(bs => {
      const list = bs ?? [];
      const block: Block = { id: `${k.kind}-${list.length + 1}`, kind: k.kind, label: k.label, description: k.description };
      // Insert just before the trailing locked block(s) (e.g. Land), so new
      // blocks land inside the editable range, never after Land.
      let insertAt = list.length;
      while (insertAt > 0 && list[insertAt - 1]?.locked) insertAt--;
      const next = [...list];
      next.splice(insertAt, 0, block);
      return next;
    });
  };

  // Drag-and-drop reordering (respects locked bookends).
  const onDrop = (target: number) => {
    setBlocks(bs => {
      if (!bs || dragIndex === null) return bs;
      const from = dragIndex;
      if (bs[from]?.locked || bs[target]?.locked) return bs;
      const [lo, hi] = movableBounds(bs);
      const dest = Math.max(lo, Math.min(hi, target));
      if (from === dest) return bs;
      const next = [...bs];
      const [moved] = next.splice(from, 1);
      next.splice(dest, 0, moved!);
      return next;
    });
    setDragIndex(null);
  };

  // Step 2: run the (edited) block list.
  const runPipeline = async () => {
    if (!newIdea.trim() || !blocks || blocks.length === 0) return;
    setCreating(true);
    try {
      const resp = await fetch('/api/projects', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          idea: newIdea.trim(),
          name: sessionName.trim() || undefined,
          blocks: blocks.map(b => ({ id: b.id, kind: b.kind, label: b.label })),
        }),
      });
      if (!resp.ok) {
        const err = await resp.json().catch(() => ({}));
        alert(err.error ?? 'Failed to create project');
        setCreating(false);
        return;
      }
      const data = await resp.json();
      navigate(`/projects/${data.name}`);
    } catch {}
    setCreating(false);
  };

  return (
    <div className="max-w-3xl mx-auto p-8">
      <h1 className="text-xl font-medium text-neutral-100 mb-8">Projects</h1>

      {/* Step 1: describe the task (full prompt) → enrich → plan */}
      <div className="mb-4 space-y-3">
        {/* Full prompt — multi-line so the whole thing is visible/editable */}
        <textarea
          className="w-full bg-neutral-900 border border-neutral-700 rounded-lg px-4 py-3 text-base text-neutral-200 placeholder-neutral-600 resize-y min-h-[120px] focus:outline-none focus:border-blue-500"
          placeholder="Describe your task in as much detail as you like…"
          value={newIdea}
          onChange={e => { setNewIdea(e.target.value); setBlocks(null); }}
          disabled={creating || enriching}
        />

        {/* Enriched-prompt proposal awaiting approval */}
        {enriched !== null && (
          <div className="border border-blue-500/40 rounded-lg p-3 bg-blue-500/5 space-y-2">
            <div className="text-sm text-blue-300 flex items-center gap-2">✨ Enriched prompt</div>
            <div className="text-sm text-neutral-300 whitespace-pre-wrap max-h-64 overflow-y-auto">{enriched}</div>
            <div className="flex gap-2 justify-end">
              <button
                onClick={() => setEnriched(null)}
                className="px-3 py-1.5 text-sm text-neutral-400 hover:text-neutral-200"
              >
                Discard
              </button>
              <button
                onClick={() => { setNewIdea(enriched); setEnriched(null); setBlocks(null); }}
                className="px-4 py-1.5 text-sm bg-blue-600 hover:bg-blue-500 text-white rounded-lg"
              >
                Use this prompt
              </button>
            </div>
          </div>
        )}

        {/* Enrich + Plan it, side by side */}
        <div className="flex justify-end gap-2">
          <button
            onClick={enrich}
            disabled={enriching || creating || !newIdea.trim()}
            title="Enrich prompt — rewrite it into a clearer, more complete prompt"
            className="flex items-center gap-2 px-4 py-2.5 text-base border border-neutral-700 hover:border-blue-500 text-neutral-300 hover:text-blue-300 disabled:opacity-30 rounded-lg transition-colors"
          >
            {enriching
              ? <><span className="inline-block w-4 h-4 border-2 border-blue-400 border-t-transparent rounded-full animate-spin" /> Enriching…</>
              : <>✨ Enrich</>}
          </button>
          <button
            onClick={planIt}
            disabled={planning || creating || !newIdea.trim()}
            className="px-6 py-2.5 text-base bg-blue-600 hover:bg-blue-500 disabled:bg-neutral-700 disabled:text-neutral-500 text-white rounded-lg transition-colors"
          >
            {planning ? 'Planning…' : 'Plan it'}
          </button>
        </div>
      </div>

      {/* Step 2: name the session + review/edit the proposed pipeline, then run */}
      {blocks && (
        <div className="mb-8 border border-neutral-800 rounded-lg p-4 bg-neutral-900/40">
          {/* Session name (optional) — sits above the pipeline */}
          <input
            type="text"
            className="w-full bg-neutral-900 border border-neutral-700 rounded-lg px-4 py-2 text-base text-neutral-200 placeholder-neutral-600 focus:outline-none focus:border-blue-500 mb-4"
            placeholder="Session name (optional)"
            value={sessionName}
            onChange={e => setSessionName(e.target.value)}
            disabled={creating}
          />
          <div className="text-sm text-neutral-400 mb-3">
            Proposed pipeline {reason && <span className="text-neutral-600">— {reason}</span>}
          </div>

          {blocks.length === 0 && (
            <div className="text-sm text-neutral-600 mb-3">No blocks. Add some below.</div>
          )}

          {/* Pipeline: each block as a row with its description inline */}
          <div className="space-y-2 mb-4">
            {blocks.map((b, i) => (
              <div
                key={`${b.id}-${i}`}
                draggable={!b.locked}
                onDragStart={() => !b.locked && setDragIndex(i)}
                onDragOver={e => { if (!b.locked) e.preventDefault(); }}
                onDrop={() => onDrop(i)}
                className={`flex items-center gap-3 border rounded-lg px-3 py-2.5 ${
                  b.locked
                    ? 'bg-neutral-800/40 border-neutral-700/60'
                    : 'bg-neutral-900 border-neutral-700 cursor-grab active:cursor-grabbing hover:border-neutral-600'
                } ${dragIndex === i ? 'opacity-50' : ''}`}
              >
                <span className="text-neutral-600 select-none w-4 text-center" title={b.locked ? 'Fixed step' : 'Drag to reorder'}>
                  {b.locked ? '🔒' : '⠿'}
                </span>
                <span className="text-neutral-600 text-sm w-5 text-right">{i + 1}</span>
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="text-neutral-100">{b.label}</span>
                    {b.checkpoint && <span className="text-xs px-1.5 py-0.5 rounded bg-amber-900/40 text-amber-400">review</span>}
                    {b.optional && <span className="text-xs px-1.5 py-0.5 rounded bg-neutral-800 text-neutral-500">optional</span>}
                    {b.locked && <span className="text-xs px-1.5 py-0.5 rounded bg-neutral-800 text-neutral-500">fixed</span>}
                  </div>
                  {b.description && <div className="text-xs text-neutral-500 mt-0.5 truncate">{b.description}</div>}
                </div>
                <div className="flex items-center gap-1">
                  <button onClick={() => move(i, -1)} disabled={b.locked} className="px-2 py-1 text-neutral-500 hover:text-neutral-200 disabled:opacity-20">↑</button>
                  <button onClick={() => move(i, 1)} disabled={b.locked} className="px-2 py-1 text-neutral-500 hover:text-neutral-200 disabled:opacity-20">↓</button>
                  <button onClick={() => remove(i)} disabled={b.locked} className="px-2 py-1 text-neutral-500 hover:text-red-400 disabled:opacity-20">✕</button>
                </div>
              </div>
            ))}
          </div>

          {/* Block library: browsable cards grouped by category, with descriptions */}
          <div className="mb-4">
            <button
              onClick={() => setShowLibrary(v => !v)}
              className="text-sm text-blue-400 hover:text-blue-300 mb-2"
            >
              {showLibrary ? '▾ Hide block library' : '▸ Add a block from the library'}
            </button>
            {showLibrary && (
              <div className="space-y-4 border border-neutral-800 rounded-lg p-3 bg-neutral-950/40">
                {CATEGORY_ORDER.filter(cat => palette.some(p => p.category === cat)).map(cat => (
                  <div key={cat}>
                    <div className="text-xs uppercase tracking-wide text-neutral-600 mb-2">{cat}</div>
                    <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
                      {palette.filter(p => p.category === cat).map(p => {
                        const count = (blocks ?? []).filter(b => b.kind === p.kind).length;
                        return (
                          <button
                            key={p.kind}
                            onClick={() => addBlock(p)}
                            className="text-left p-2.5 rounded-lg border border-neutral-800 bg-neutral-900 hover:border-blue-600 hover:bg-blue-500/5 transition-colors group"
                          >
                            <div className="flex items-center gap-2">
                              <span className="text-neutral-200 group-hover:text-blue-300">{p.label}</span>
                              {count > 0 && <span className="text-xs text-neutral-600">×{count} in pipeline</span>}
                              <span className="ml-auto text-neutral-600 group-hover:text-blue-400">+ add</span>
                            </div>
                            <div className="text-xs text-neutral-500 mt-1">{p.description}</div>
                          </button>
                        );
                      })}
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>

          <div className="flex justify-end">
            <button
              onClick={runPipeline}
              disabled={creating || blocks.length === 0}
              className="px-6 py-2.5 text-base bg-green-600 hover:bg-green-500 disabled:bg-neutral-700 disabled:text-neutral-500 text-white rounded-lg font-medium transition-colors"
            >
              {creating ? 'Starting…' : 'Run pipeline ▶'}
            </button>
          </div>
        </div>
      )}

      {/* Project List */}
      {projects.length === 0 ? (
        <p className="text-neutral-600 text-base">No projects yet. Describe an idea above to start.</p>
      ) : (
        <div className="space-y-2">
          {projects.map(project => (
            <button
              key={project.name}
              onClick={() => navigate(`/projects/${project.name}`)}
              className="w-full text-left px-4 py-3 rounded-lg hover:bg-neutral-800/50 transition-colors flex items-center gap-4"
            >
              <div className="flex-1 min-w-0">
                <div className="text-base text-neutral-200 truncate">{project.name}</div>
                <div className="text-sm text-neutral-500 truncate mt-0.5">{project.idea}</div>
              </div>
              <PhaseTag phase={project.phase} />
              <span className="text-sm text-neutral-600 whitespace-nowrap">
                {timeAgo(project.updated_at)}
              </span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
