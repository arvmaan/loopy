import { useParams } from 'react-router-dom';
import { useState, useEffect, useRef, Fragment } from 'react';
import Markdown from 'react-markdown';
import { useEngine } from '../hooks/useEngine';
import type { Phase, StageState, TrackState, ReviewDiff, ReviewComment } from '../types/v2';

function phaseLabel(phase: Phase): string {
  const labels: Record<Phase, string> = {
    initializing: 'Starting...',
    scanning: 'Scanning codebase',
    planning: 'Creating plan',
    awaiting_plan_review: 'Review plan',
    setting_up_workspaces: 'Setting up workspaces',
    running_tracks: 'Running tracks',
    awaiting_code_review: 'Flight Check',
    awaiting_test_plan: 'Test Flight — review plan',
    test_flying: 'Test Flight — running',
    awaiting_test_review: 'Test Flight — review results',
    landing: 'Landing',
    complete: 'Complete',
    failed: 'Failed',
  };
  return labels[phase];
}

function StatusIcon({ status }: { status: 'pending' | 'running' | 'complete' | 'failed' }) {
  switch (status) {
    case 'complete': return <span className="text-green-400 text-lg">✓</span>;
    case 'running': return <span className="text-blue-400 animate-pulse text-lg">●</span>;
    case 'failed': return <span className="text-red-400 text-lg">✗</span>;
    case 'pending': return <span className="text-neutral-600 text-lg">○</span>;
  }
}

function StagePanel({ stage, expanded, children, title, subtitle }: {
  stage: StageState;
  expanded: boolean;
  children?: React.ReactNode;
  title?: string;     // display name override (e.g. flight-themed block name)
  subtitle?: string;  // small description shown under the title
}) {
  // Use completed_at only when the stage is actually finished; while running,
  // measure against now. Clamp to >= 0 to avoid garbage from stale timestamps.
  const elapsedMs = stage.started_at
    ? (stage.status === 'complete' && stage.completed_at
        ? new Date(stage.completed_at).getTime() - new Date(stage.started_at).getTime()
        : Date.now() - new Date(stage.started_at).getTime())
    : null;
  const elapsed = elapsedMs != null ? formatDuration(Math.max(0, elapsedMs)) : null;

  return (
    <div className={`border-l-2 pl-5 py-3 ${
      stage.status === 'running' ? 'border-blue-400' :
      stage.status === 'complete' ? 'border-green-400/40' :
      stage.status === 'failed' ? 'border-red-400' :
      'border-neutral-700'
    }`}>
      <div className="flex items-center gap-3">
        <StatusIcon status={stage.status} />
        <div className="flex flex-col">
          <span className={`text-base ${stage.status === 'pending' ? 'text-neutral-500' : 'text-neutral-100'}`}>
            {title ?? stageName(stage.id)}
          </span>
          {subtitle && <span className="text-xs text-neutral-500 mt-0.5">{subtitle}</span>}
        </div>
        {elapsed && <span className="text-neutral-500 ml-auto text-sm">{elapsed}</span>}
      </div>
      {expanded && children && (
        <div className="mt-3 ml-7">{children}</div>
      )}
    </div>
  );
}

function stageName(id: string): string {
  const names: Record<string, string> = {
    idea: 'Idea',
    scan: 'Scan',
    plan: 'Plan',
    orbital_lanes: 'Tracks',
    flight_check: 'Flight Check',
    test_flight: 'Test Flight',
    land: 'Land',
  };
  return names[id] || id;
}

function formatDuration(ms: number): string {
  const secs = Math.floor(ms / 1000);
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  return `${mins}m ${secs % 60}s`;
}

function StreamingLog({ logs, fullscreen }: { logs: Array<{ timestamp: string; level: string; message: string }>; fullscreen?: boolean }) {
  const endRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    endRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [logs.length]);

  return (
    <div className={`bg-neutral-900 rounded-lg p-4 overflow-y-auto text-sm font-mono ${fullscreen ? 'h-[calc(100vh-200px)]' : 'max-h-64'}`}>
      {logs.length === 0 && <span className="text-neutral-600">Waiting for output...</span>}
      {logs.map((log, i) => (
        <div key={i} className={`leading-relaxed ${
          log.level === 'error' ? 'text-red-400' :
          log.level === 'warn' ? 'text-yellow-400' :
          'text-neutral-400'
        }`}>
          {log.message}
        </div>
      ))}
      <div ref={endRef} />
    </div>
  );
}

const SPACE_PHRASES = [
  'Configuring rocket ship design',
  'Calibrating orbital trajectory',
  'Spinning up thrusters',
  'Triangulating star charts',
  'Pressurizing the cabin',
  'Aligning solar panels',
  'Charting the asteroid belt',
  'Reticulating splines',
  'Engaging warp coils',
  'Scanning for space debris',
  'Plotting the flight path',
  'Warming up the ion drive',
  'Decoding telemetry streams',
  'Syncing with mission control',
  'Adjusting the gyroscopes',
  'Polishing the viewport',
  'Fueling the booster stage',
  'Running pre-flight checks',
  'Deploying the antenna array',
  'Stabilizing the gravity well',
  // more
  'Untangling the tether lines',
  'Defrosting the cryo pods',
  'Negotiating with the autopilot',
  'Counting the moons',
  'Recalibrating the flux capacitor',
  'Dusting off the solar sails',
  'Feeding the hull microbes',
  'Tuning the subspace radio',
  'Mapping the nebula',
  'Greasing the airlock hinges',
  'Consulting the navigation droid',
  'Buffering the wormhole',
  'Inflating the escape pods',
  'Sorting the space groceries',
  'Rebooting the gravity boots',
  'Whispering to the reactor core',
  'Aligning with true north star',
  'Coiling the plasma conduits',
  'Sweeping for micrometeorites',
  'Brewing the astronaut coffee',
  'Calibrating the docking clamps',
  'Charging the deflector shields',
  'Lubricating the landing gear',
  'Decrypting alien signals',
  'Balancing the antimatter',
  'Spooling up the hyperdrive',
  'Checking the oxygen levels',
  'Polishing the heat shield',
  'Synchronizing the star map',
  'Venting the excess plasma',
];

const SPINNER_FRAMES = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

// Spinner + progress-bar color cycles with each phrase change, just for fun.
// Paired text/bg classes (literal strings so Tailwind doesn't purge them).
const ACTIVITY_COLORS = [
  { text: 'text-blue-400', bar: 'bg-blue-500/70' },
  { text: 'text-purple-400', bar: 'bg-purple-500/70' },
  { text: 'text-cyan-400', bar: 'bg-cyan-500/70' },
  { text: 'text-emerald-400', bar: 'bg-emerald-500/70' },
  { text: 'text-amber-400', bar: 'bg-amber-500/70' },
  { text: 'text-pink-400', bar: 'bg-pink-500/70' },
  { text: 'text-indigo-400', bar: 'bg-indigo-500/70' },
  { text: 'text-teal-400', bar: 'bg-teal-500/70' },
];

function ActivityBar({ logCount }: { logCount: number }) {
  const [frame, setFrame] = useState(0);
  const [phraseIdx, setPhraseIdx] = useState(0);
  const [colorIdx, setColorIdx] = useState(0);

  // Spinner animation
  useEffect(() => {
    const id = setInterval(() => setFrame(f => (f + 1) % SPINNER_FRAMES.length), 100);
    return () => clearInterval(id);
  }, []);

  // Rotate phrase + color every ~7s (slower, calmer)
  useEffect(() => {
    const id = setInterval(() => {
      setPhraseIdx(() => Math.floor(Math.random() * SPACE_PHRASES.length));
      setColorIdx(c => (c + 1) % ACTIVITY_COLORS.length);
    }, 7000);
    return () => clearInterval(id);
  }, []);

  const color = ACTIVITY_COLORS[colorIdx] ?? ACTIVITY_COLORS[0]!;

  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2 text-sm">
        <span className={`font-mono transition-colors duration-500 ${color.text}`}>{SPINNER_FRAMES[frame]}</span>
        <span className="text-neutral-400">{SPACE_PHRASES[phraseIdx]}…</span>
        <span className="text-xs text-neutral-600 ml-auto">{logCount} events</span>
      </div>
      {/* Indeterminate animated progress bar — color cycles with the spinner */}
      <div className="h-1 bg-neutral-800 rounded-full overflow-hidden relative">
        <div className={`absolute h-full w-1/3 rounded-full loopy-indeterminate transition-colors duration-500 ${color.bar}`} />
      </div>
    </div>
  );
}

function ReviewPanel({ onApprove, onReject, content }: {
  onApprove: () => void;
  onReject: (feedback: string) => void;
  content: string | null;
}) {
  const [feedback, setFeedback] = useState('');

  return (
    <div className="space-y-4">
      {content ? (
        <div className="bg-neutral-900 rounded-lg p-5 max-h-[500px] overflow-y-auto text-sm text-neutral-300 leading-relaxed prose prose-invert prose-sm max-w-none">
          <Markdown>{content}</Markdown>
        </div>
      ) : (
        <div className="bg-neutral-900 rounded-lg p-5 text-sm text-neutral-500">
          Loading plan content...
        </div>
      )}
      <div className="bg-neutral-800/50 rounded-lg p-4 space-y-3">
        <textarea
          className="w-full bg-neutral-900 border border-neutral-700 rounded-lg p-3 text-base text-neutral-200 placeholder-neutral-600 resize-y min-h-[80px] focus:outline-none focus:border-blue-500"
          placeholder="Feedback (optional — what to change)..."
          value={feedback}
          onChange={e => setFeedback(e.target.value)}
          onKeyDown={e => {
            if (e.key === 'Enter' && e.metaKey && feedback.trim()) {
              onReject(feedback.trim());
              setFeedback('');
            }
          }}
        />
        <div className="flex gap-3 justify-end">
          <button
            onClick={() => { onReject(feedback.trim() || 'Please revise'); setFeedback(''); }}
            className="px-4 py-2 text-sm bg-neutral-700 hover:bg-neutral-600 text-neutral-200 rounded-lg transition-colors"
          >
            Request Changes
          </button>
          <button
            onClick={onApprove}
            className="px-5 py-2 text-sm bg-green-600 hover:bg-green-500 text-white rounded-lg font-medium transition-colors"
          >
            Approve ▶
          </button>
        </div>
      </div>
    </div>
  );
}

// Build the structured feedback markdown sent to the tracks on "Request changes".
function buildReviewFeedback(comments: ReviewComment[], summary: string): string {
  let out = '';
  if (summary.trim()) {
    out += `## Summary\n\n${summary.trim()}\n\n`;
  }
  if (comments.length > 0) {
    out += `## Inline comments\n\n`;
    // Group comments by track → package → file for a readable handoff.
    const byTrack: Record<string, ReviewComment[]> = {};
    for (const c of comments) (byTrack[c.track] ??= []).push(c);
    for (const [track, cs] of Object.entries(byTrack)) {
      out += `### Track: ${track}\n\n`;
      for (const c of cs) {
        out += `- \`${c.package}/${c.file}\`: ${c.text}\n`;
      }
      out += '\n';
    }
  }
  return out.trim() || 'Please revise.';
}

// Flight Check — guided review of the committed changes, grouped track → package,
// with inline comments + an overall summary. "Request changes" routes the bundled
// feedback back to the tracks (additively); "Approve" continues the pipeline.
function FlightCheckPanel({ projectName, onApprove, onReject }: {
  projectName: string;
  onApprove: (opts?: { testFlight?: boolean }) => void;
  onReject: (feedback: string) => void;
}) {
  const [diff, setDiff] = useState<ReviewDiff | null>(null);
  const [fetched, setFetched] = useState(false);
  const [selTrack, setSelTrack] = useState(0);
  const [selPkg, setSelPkg] = useState(0);
  const [comments, setComments] = useState<ReviewComment[]>([]);
  const [summary, setSummary] = useState('');
  const [commenting, setCommenting] = useState<{ file: string; line: number } | null>(null);
  const [draft, setDraft] = useState('');

  useEffect(() => {
    if (fetched) return;
    setFetched(true);
    fetch(`/api/projects/${projectName}/review-diff`)
      .then(r => r.json())
      .then(d => setDiff(d))
      .catch(() => {});
  }, [projectName, fetched]);

  const groups = diff?.groups ?? [];
  const totalFiles = groups.reduce((n, g) => n + g.file_count, 0);
  const track = groups[selTrack];
  const pkg = track?.packages[selPkg];

  const addComment = (file: string, line: number, text: string) => {
    if (!track || !pkg || !text.trim()) return;
    setComments(cs => [...cs, { track: track.track, package: pkg.package, file, line, text: text.trim() }]);
    setCommenting(null);
    setDraft('');
  };

  if (diff && totalFiles === 0) {
    return (
      <div className="space-y-4">
        <div className="text-sm text-neutral-500">No committed changes detected across any track.</div>
        <div className="flex gap-3 justify-end">
          <button onClick={() => onApprove({ testFlight: true })} className="px-4 py-2 text-sm bg-blue-600 hover:bg-blue-500 text-white rounded-lg font-medium">Approve & Test Flight 🚀</button>
          <button onClick={() => onApprove({ testFlight: false })} className="px-5 py-2 text-sm bg-green-600 hover:bg-green-500 text-white rounded-lg font-medium">Approve & Land ▶</button>
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-3 text-sm text-neutral-500">
        <span>{totalFiles} files changed across {groups.length} tracks</span>
        {comments.length > 0 && <span className="text-amber-400">· {comments.length} comment{comments.length > 1 ? 's' : ''}</span>}
      </div>

      <div className="flex gap-4">
        {/* Track + package navigator */}
        <div className="w-60 shrink-0 space-y-3">
          {groups.map((g, gi) => (
            <div key={g.track}>
              <button
                onClick={() => { setSelTrack(gi); setSelPkg(0); }}
                className={`w-full text-left text-sm font-medium px-2 py-1 rounded flex justify-between ${gi === selTrack ? 'text-neutral-100' : 'text-neutral-400 hover:text-neutral-200'}`}
              >
                <span>{g.track}</span>
                <span className="text-neutral-600 text-xs">{g.file_count}</span>
              </button>
              {gi === selTrack && g.packages.map((p, pi) => (
                <button
                  key={p.package}
                  onClick={() => setSelPkg(pi)}
                  className={`w-full text-left text-xs px-3 py-1 rounded truncate ${pi === selPkg ? 'bg-neutral-800 text-blue-300' : 'text-neutral-500 hover:text-neutral-300'}`}
                >
                  {p.package} <span className="text-neutral-600">({p.files.length})</span>
                </button>
              ))}
            </div>
          ))}
        </div>

        {/* Diff view for the selected package */}
        <div className="flex-1 min-w-0 space-y-4">
          {pkg && pkg.files.length === 0 && (
            <div className="text-sm text-neutral-600">No changes in this package.</div>
          )}
          {pkg?.files.map(file => {
            let lineNo = -1;
            return (
              <div key={file.path} className="bg-neutral-900 rounded-lg overflow-hidden">
                <div className="px-3 py-2 text-xs font-mono text-neutral-300 border-b border-neutral-800">{file.path}</div>
                <div className="p-2 overflow-x-auto text-xs font-mono">
                  {file.hunks.map((hunk, hi) => (
                    <div key={hi} className="mb-2">
                      <div className="text-neutral-600">{hunk.header}</div>
                      {hunk.lines.map((line, li) => {
                        lineNo++;
                        const thisLine = lineNo;
                        const lineComments = comments.filter(c => c.file === file.path && c.line === thisLine && c.track === track?.track && c.package === pkg.package);
                        return (
                          <div key={li}>
                            <div
                              className={`group flex cursor-pointer ${
                                line.kind === 'Added' ? 'text-green-400 bg-green-950/30' :
                                line.kind === 'Removed' ? 'text-red-400 bg-red-950/30' :
                                'text-neutral-500'
                              } hover:bg-neutral-800/60`}
                              onClick={() => { setCommenting({ file: file.path, line: thisLine }); setDraft(''); }}
                            >
                              <span className="select-none opacity-0 group-hover:opacity-100 text-blue-400 px-1">+</span>
                              <span className="whitespace-pre">{line.kind === 'Added' ? '+' : line.kind === 'Removed' ? '-' : ' '}{line.content}</span>
                            </div>
                            {lineComments.map((c, ci) => (
                              <div key={ci} className="ml-6 my-1 bg-amber-500/10 border-l-2 border-amber-500 px-2 py-1 text-amber-200/90 rounded-r">
                                {c.text}
                              </div>
                            ))}
                            {commenting?.file === file.path && commenting?.line === thisLine && (
                              <div className="ml-6 my-1 flex gap-2">
                                <input
                                  autoFocus
                                  value={draft}
                                  onChange={e => setDraft(e.target.value)}
                                  onKeyDown={e => { if (e.key === 'Enter') addComment(file.path, thisLine, draft); if (e.key === 'Escape') setCommenting(null); }}
                                  placeholder="Add a comment, Enter to save…"
                                  className="flex-1 bg-neutral-950 border border-neutral-700 rounded px-2 py-1 text-neutral-200 focus:outline-none focus:border-amber-500"
                                />
                                <button onClick={() => addComment(file.path, thisLine, draft)} className="px-2 py-1 bg-amber-600/80 hover:bg-amber-500 text-white rounded">Save</button>
                              </div>
                            )}
                          </div>
                        );
                      })}
                    </div>
                  ))}
                </div>
              </div>
            );
          })}
        </div>
      </div>

      {/* Summary + actions */}
      <div className="bg-neutral-800/50 rounded-lg p-4 space-y-3">
        <textarea
          className="w-full bg-neutral-900 border border-neutral-700 rounded-lg p-3 text-base text-neutral-200 placeholder-neutral-600 resize-y min-h-[80px] focus:outline-none focus:border-blue-500"
          placeholder="Overall review summary (optional). Click any diff line above to add an inline comment."
          value={summary}
          onChange={e => setSummary(e.target.value)}
        />
        <div className="flex items-center gap-3 justify-end">
          {comments.length > 0 && (
            <button onClick={() => setComments([])} className="text-xs text-neutral-500 hover:text-neutral-300 mr-auto">Clear {comments.length} comment{comments.length > 1 ? 's' : ''}</button>
          )}
          <button
            onClick={() => onReject(buildReviewFeedback(comments, summary))}
            className="px-4 py-2 text-sm bg-amber-700 hover:bg-amber-600 text-amber-50 rounded-lg transition-colors"
          >
            Request changes ↻
          </button>
          <button
            onClick={() => onApprove({ testFlight: true })}
            title="Deploy to a beta/test environment and run an E2E test plan before landing"
            className="px-4 py-2 text-sm bg-blue-600 hover:bg-blue-500 text-white rounded-lg font-medium transition-colors"
          >
            Approve & Test Flight 🚀
          </button>
          <button
            onClick={() => onApprove({ testFlight: false })}
            className="px-5 py-2 text-sm bg-green-600 hover:bg-green-500 text-white rounded-lg font-medium transition-colors"
          >
            Approve & Land ▶
          </button>
        </div>
      </div>
    </div>
  );
}

// Test Flight — the opt-in beta-test stage. Three sub-phases:
//  - awaiting_test_plan: edit + approve the draft TESTING-PLAN.md
//  - test_flying: the agent deploys to beta + runs E2E (stream logs)
//  - awaiting_test_review: accept (→ Land) or request changes (→ Tracks)
function TestFlightPanel({ projectName, phase, logs, onApprove, onReject }: {
  projectName: string;
  phase: Phase;
  logs: Array<{ timestamp: string; level: string; message: string }>;
  onApprove: () => void;
  onReject: (feedback: string) => void;
}) {
  const [plan, setPlan] = useState<string | null>(null);
  const [fetched, setFetched] = useState(false);
  const [saving, setSaving] = useState(false);
  const [savedAt, setSavedAt] = useState<number | null>(null);
  const [feedback, setFeedback] = useState('');

  useEffect(() => {
    if (fetched || phase !== 'awaiting_test_plan') return;
    setFetched(true);
    fetch(`/api/projects/${projectName}/test-plan`)
      .then(r => r.json())
      .then(d => setPlan(d.content ?? ''))
      .catch(() => setPlan(''));
  }, [projectName, phase, fetched]);

  const savePlan = async () => {
    setSaving(true);
    try {
      await fetch(`/api/projects/${projectName}/test-plan`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ content: plan ?? '' }),
      });
      setSavedAt(Date.now());
    } finally {
      setSaving(false);
    }
  };

  // 1. Plan review — editable TESTING-PLAN.md
  if (phase === 'awaiting_test_plan') {
    return (
      <div className="space-y-3">
        <p className="text-sm text-neutral-400">
          Edit the test plan: how to deploy to beta and what to verify. Loopy runs this against a
          beta/test environment — it won't touch production.
        </p>
        <textarea
          className="w-full bg-neutral-950 border border-neutral-700 rounded-lg p-3 text-sm font-mono text-neutral-200 resize-y min-h-[320px] focus:outline-none focus:border-blue-500"
          value={plan ?? ''}
          onChange={e => { setPlan(e.target.value); setSavedAt(null); }}
          placeholder={plan === null ? 'Loading plan…' : ''}
        />
        <div className="flex items-center gap-3 justify-end">
          {savedAt && <span className="text-xs text-green-400 mr-auto">Saved ✓</span>}
          <button onClick={savePlan} disabled={saving} className="px-3 py-2 text-sm bg-neutral-700 hover:bg-neutral-600 text-neutral-200 rounded-lg">
            {saving ? 'Saving…' : 'Save draft'}
          </button>
          <button
            onClick={async () => { await savePlan(); onApprove(); }}
            className="px-5 py-2 text-sm bg-green-600 hover:bg-green-500 text-white rounded-lg font-medium"
          >
            Approve & run 🚀
          </button>
        </div>
      </div>
    );
  }

  // 2. Running — stream the agent output
  if (phase === 'test_flying') {
    return (
      <div className="space-y-2">
        <ActivityBar logCount={logs.length} />
        <StreamingLog logs={logs} />
      </div>
    );
  }

  // 3. Results review — accept or request changes
  if (phase === 'awaiting_test_review') {
    return (
      <div className="space-y-3">
        <p className="text-sm text-neutral-400">
          Test flight finished. Review the output above. If it looks good, land it. If you found
          issues, request changes — feedback goes back to the tracks and they iterate on top of the
          existing work.
        </p>
        <StreamingLog logs={logs} />
        <textarea
          className="w-full bg-neutral-900 border border-neutral-700 rounded-lg p-3 text-base text-neutral-200 placeholder-neutral-600 resize-y min-h-[80px] focus:outline-none focus:border-blue-500"
          placeholder="What went wrong / what to fix (only needed if requesting changes)…"
          value={feedback}
          onChange={e => setFeedback(e.target.value)}
        />
        <div className="flex gap-3 justify-end">
          <button
            onClick={() => onReject(feedback.trim() || 'Issues found during test flight; please revise.')}
            className="px-4 py-2 text-sm bg-amber-700 hover:bg-amber-600 text-amber-50 rounded-lg"
          >
            Request changes ↻
          </button>
          <button onClick={onApprove} className="px-5 py-2 text-sm bg-green-600 hover:bg-green-500 text-white rounded-lg font-medium">
            Looks good — Land ▶
          </button>
        </div>
      </div>
    );
  }

  return null;
}

// Fetches and renders a block's produced markdown artifact (design-spec, plan, etc.)
// so the user can read it at a review checkpoint.
function BlockOutput({ projectName, blockId }: { projectName: string; blockId: string }) {
  const [content, setContent] = useState<string | null>(null);
  const [file, setFile] = useState<string>('');
  const [loaded, setLoaded] = useState(false);
  useEffect(() => {
    fetch(`/api/projects/${projectName}/blocks/${blockId}/output`)
      .then(r => r.json())
      .then(d => { setContent(d.content ?? null); setFile(d.file ?? ''); setLoaded(true); })
      .catch(() => setLoaded(true));
  }, [projectName, blockId]);
  if (!loaded) return <div className="text-sm text-neutral-600">Loading output…</div>;
  if (!content) return null;
  return (
    <div className="bg-neutral-950 rounded-lg border border-neutral-800">
      {file && <div className="px-3 py-1.5 text-xs font-mono text-neutral-500 border-b border-neutral-800">{file}</div>}
      <div className="p-4 max-h-[420px] overflow-y-auto prose prose-invert prose-sm max-w-none">
        <Markdown>{content}</Markdown>
      </div>
    </div>
  );
}

// Renders a generic (non-POC) linear pipeline run — its blocks as steps, matching
// the POC StagePanel look. Active block shows the activity bar + log; a paused
// checkpoint shows the block's output + approve / request-changes.
function LinearPipeline({ projectName, linear, logs, onApprove, onReject, onExpand }: {
  projectName: string;
  linear: import('../types/v2').LinearProgress;
  logs: Array<{ timestamp: string; level: string; message: string }>;
  onApprove: () => void;
  onReject: (feedback: string) => void;
  onExpand: (blockId: string) => void;
}) {
  const [feedback, setFeedback] = useState('');
  const statusOf = (id: string): 'pending' | 'running' | 'complete' | 'failed' => {
    if (linear.completed.includes(id)) return 'complete';
    if (linear.current === id) return 'running';
    return 'pending';
  };
  return (
    <div className="space-y-0">
      {linear.stages.map(s => {
        const status = statusOf(s.id);
        const isPaused = linear.paused === s.id;
        const isActiveRun = linear.current === s.id && !isPaused && !linear.done;
        return (
          <StagePanel
            key={s.id}
            stage={{ id: s.id, status, loop_id: null, loop_pid: null, started_at: null, completed_at: null, error: null } as unknown as StageState}
            expanded={isActiveRun || isPaused}
            title={s.label}
            subtitle={s.description}
          >
            {isActiveRun && (
              <div className="space-y-2">
                <ActivityBar logCount={logs.length} />
                <StreamingLog logs={logs} />
                <button
                  onClick={() => onExpand(s.id)}
                  className="text-xs text-neutral-500 hover:text-neutral-300 transition-colors"
                >
                  Expand ↗
                </button>
              </div>
            )}
            {isPaused && (
              <div className="bg-neutral-800/50 rounded-lg p-4 space-y-3">
                <p className="text-sm text-neutral-400">
                  {s.label} is ready for review. Read its output below, then approve to continue
                  or request changes to send feedback back and iterate.
                </p>
                <BlockOutput projectName={projectName} blockId={s.id} />
                <StreamingLog logs={logs} />
                <textarea
                  className="w-full bg-neutral-900 border border-neutral-700 rounded-lg p-3 text-base text-neutral-200 placeholder-neutral-600 resize-y min-h-[80px] focus:outline-none focus:border-blue-500"
                  placeholder="What to change (only needed when requesting changes)…"
                  value={feedback}
                  onChange={e => setFeedback(e.target.value)}
                />
                <div className="flex gap-3 justify-end">
                  <button
                    onClick={() => { onReject(feedback.trim() || 'Please revise.'); setFeedback(''); }}
                    className="px-4 py-2 text-sm bg-amber-700 hover:bg-amber-600 text-amber-50 rounded-lg"
                  >
                    Request changes ↻
                  </button>
                  <button onClick={onApprove} className="px-5 py-2 text-sm bg-green-600 hover:bg-green-500 text-white rounded-lg font-medium">
                    Approve ▶
                  </button>
                </div>
              </div>
            )}
          </StagePanel>
        );
      })}
    </div>
  );
}

// Small inline spinner for a single running track.
function MiniSpinner() {
  const [frame, setFrame] = useState(0);
  useEffect(() => {
    const id = setInterval(() => setFrame(f => (f + 1) % SPINNER_FRAMES.length), 100);
    return () => clearInterval(id);
  }, []);
  return <span className="font-mono text-blue-400">{SPINNER_FRAMES[frame]}</span>;
}

// Tails a single track's Ralph log by polling the per-track log endpoint.
function TrackLogTail({ projectName, trackId, columnar }: {
  projectName: string;
  trackId: string;
  columnar?: boolean;
}) {
  const [lines, setLines] = useState<string[]>([]);
  const endRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let active = true;
    const poll = () => {
      fetch(`/api/projects/${projectName}/tracks/${trackId}/log`)
        .then(r => r.json())
        .then(d => {
          if (!active) return;
          const text: string = d.content ?? '';
          setLines(text ? text.split('\n') : []);
        })
        .catch(() => {});
    };
    poll();
    const id = setInterval(poll, 2000);
    return () => { active = false; clearInterval(id); };
  }, [projectName, trackId]);

  useEffect(() => {
    endRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [lines.length]);

  return (
    <div className={`bg-neutral-950 rounded-lg p-3 overflow-y-auto text-xs font-mono border border-neutral-800 ${columnar ? 'h-[420px]' : 'max-h-56'}`}>
      {lines.length === 0 && <span className="text-neutral-600">Waiting for output…</span>}
      {lines.map((line, i) => (
        <div key={i} className="leading-relaxed text-neutral-400 whitespace-pre-wrap break-all">{line}</div>
      ))}
      <div ref={endRef} />
    </div>
  );
}

function TrackProgressPanel({ projectName, tracks, progress }: {
  projectName: string;
  tracks: TrackState[];
  progress: Record<string, { tasks_done: number; tasks_total: number; current: string }>;
}) {
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [columnar, setColumnar] = useState(false);

  const toggle = (id: string) => setExpanded(e => ({ ...e, [id]: !e[id] }));
  const runningCount = tracks.filter(t => t.status === 'running').length;

  // Columnar view — one column per track, each tailing its own log.
  if (columnar) {
    return (
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <span className="text-sm text-neutral-500">{runningCount} running · {tracks.length} tracks</span>
          <button onClick={() => setColumnar(false)} className="text-xs text-blue-400 hover:text-blue-300">
            ▦ Stacked view
          </button>
        </div>
        <div className="flex gap-3 overflow-x-auto pb-2">
          {tracks.map(track => {
            const p = progress[track.id];
            return (
              <div key={track.id} className="flex-shrink-0 w-80 space-y-2">
                <div className="flex items-center gap-2 text-sm">
                  {track.status === 'running' ? <MiniSpinner /> : <StatusIcon status={track.status as any} />}
                  <span className="text-neutral-200 truncate">{track.name || track.id}</span>
                  {p && p.tasks_total > 0 && (
                    <span className="text-neutral-600 text-xs ml-auto">{p.tasks_done}/{p.tasks_total}</span>
                  )}
                </div>
                {p && <span className="text-neutral-500 text-xs block truncate">{p.current}</span>}
                <TrackLogTail projectName={projectName} trackId={track.id} columnar />
              </div>
            );
          })}
        </div>
      </div>
    );
  }

  // Stacked view — one row per track, expandable to tail its log.
  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <span className="text-sm text-neutral-500">{runningCount} running · {tracks.length} tracks</span>
        {tracks.length > 1 && (
          <button onClick={() => setColumnar(true)} className="text-xs text-blue-400 hover:text-blue-300">
            ▥ Columns
          </button>
        )}
      </div>
      {tracks.map(track => {
        const p = progress[track.id];
        const isRunning = track.status === 'running';
        const pct = p && p.tasks_total > 0 ? Math.round((p.tasks_done / p.tasks_total) * 100) : 0;
        return (
          <div key={track.id} className="rounded-lg border border-neutral-800 bg-neutral-900/40">
            <div className="flex items-center gap-3 text-base px-3 py-2">
              {isRunning ? <MiniSpinner /> : <StatusIcon status={track.status as any} />}
              <span className="text-neutral-200">{track.name || track.id}</span>
              <div className="ml-auto flex items-center gap-3">
                {p && (
                  <span className="text-neutral-500 text-sm">
                    {p.tasks_total > 0 && <span className="text-neutral-400">{p.tasks_done}/{p.tasks_total} · </span>}
                    {p.current}
                  </span>
                )}
                {!p && isRunning && <span className="text-neutral-500 text-sm">working…</span>}
                <button onClick={() => toggle(track.id)} className="text-xs text-blue-400 hover:text-blue-300">
                  {expanded[track.id] ? '▾ Hide log' : '▸ Tail log'}
                </button>
              </div>
            </div>
            {p && p.tasks_total > 0 && (
              <div className="h-1 mx-3 mb-2 bg-neutral-800 rounded-full overflow-hidden">
                <div className="h-full bg-blue-500/70 rounded-full transition-all duration-500" style={{ width: `${pct}%` }} />
              </div>
            )}
            {expanded[track.id] && (
              <div className="px-3 pb-3">
                <TrackLogTail projectName={projectName} trackId={track.id} />
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

export function PipelineView() {
  const { name } = useParams<{ name: string }>();
  const { state, logs, trackProgress, linear, connected, approve, reject, abort } = useEngine(name);
  const [planContent, setPlanContent] = useState<string | null>(null);
  const [planFetched, setPlanFetched] = useState(false);
  const [, setTick] = useState(0);
  const [focusedStage, setFocusedStage] = useState<string | null>(null);

  // Tick every second to keep elapsed times fresh during running stages
  useEffect(() => {
    const hasRunning = state?.stages.some(s => s.status === 'running');
    if (!hasRunning) return;
    const interval = setInterval(() => setTick(t => t + 1), 1000);
    return () => clearInterval(interval);
  }, [state?.stages.map(s => s.status).join()]);

  useEffect(() => {
    if (state?.phase === 'awaiting_plan_review') {
      const fetchPlan = () => {
        fetch(`/api/projects/${name}/plan`)
          .then(r => r.json())
          .then(data => {
            if (data.content) {
              setPlanContent(data.content);
            } else if (!planFetched) {
              // Retry after 2s if content not ready yet
              setTimeout(fetchPlan, 2000);
            }
          })
          .catch(() => {});
      };
      if (!planFetched) {
        setPlanFetched(true);
        fetchPlan();
      }
    }
    if (state?.phase === 'planning') {
      setPlanFetched(false);
      setPlanContent(null);
    }
  }, [state?.phase, name, planFetched]);

  if (!state) {
    return (
      <div className="max-w-[1600px] mx-auto p-8">
        <div className="flex items-center gap-2 text-neutral-500 text-base">
          <span className={`w-2 h-2 rounded-full ${connected ? 'bg-green-400' : 'bg-yellow-400 animate-pulse'}`} />
          {connected ? 'Loading state...' : 'Connecting to pipeline...'}
        </div>
      </div>
    );
  }

  const isAtCheckpoint = state.phase === 'awaiting_plan_review' || state.phase === 'awaiting_code_review';

  // Focused/fullscreen view for a stage. Works for POC stages AND linear blocks
  // (linear blocks aren't in state.stages, so fall back to the linear block's view).
  if (focusedStage) {
    const stage = state.stages.find(s => s.id === focusedStage);
    const linearBlock = linear?.stages.find(b => b.id === focusedStage);
    if (stage || linearBlock) {
      const elapsed = stage?.started_at
        ? formatDuration(Date.now() - new Date(stage.started_at).getTime())
        : null;
      const title = linearBlock ? linearBlock.label : stageName(stage!.id);
      const running = stage ? stage.status === 'running' : (linear?.current === focusedStage && !linear?.done);
      return (
        <div className="max-w-[1600px] mx-auto p-6">
          <div className="flex items-center gap-3 mb-4">
            <button onClick={() => setFocusedStage(null)} className="text-neutral-500 hover:text-neutral-300 text-sm">
              ← Back
            </button>
            <StatusIcon status={(stage?.status as any) ?? (running ? 'running' : 'complete')} />
            <span className="text-lg text-neutral-100">{title}</span>
            {linearBlock?.description && <span className="text-sm text-neutral-500">{linearBlock.description}</span>}
            {elapsed && <span className="text-neutral-500 text-sm">{elapsed}</span>}
          </div>
          {running && (
            <div className="mb-4"><ActivityBar logCount={logs.length} /></div>
          )}
          <StreamingLog logs={logs} fullscreen />
        </div>
      );
    }
  }

  return (
    <div className="max-w-[1600px] mx-auto p-8 space-y-2">
      {/* Header */}
      <div className="flex items-center justify-between mb-8">
        <div>
          <h1 className="text-xl font-medium text-neutral-100">{state.project_name}</h1>
          <p className="text-sm text-neutral-500 mt-1 max-w-lg truncate" title={state.idea}>
            {state.idea.length > 100 ? state.idea.slice(0, 100) + '...' : state.idea}
          </p>
        </div>
        <div className="flex items-center gap-4">
          <span className={`text-sm px-3 py-1 rounded-lg ${
            state.phase === 'complete' ? 'bg-green-900/50 text-green-400' :
            state.phase === 'failed' ? 'bg-red-900/50 text-red-400' :
            isAtCheckpoint ? 'bg-yellow-900/50 text-yellow-400' :
            'bg-blue-900/50 text-blue-400'
          }`}>
            {phaseLabel(state.phase)}
          </span>
          {state.phase !== 'complete' && state.phase !== 'failed' && (
            <button
              onClick={abort}
              className="text-sm text-neutral-500 hover:text-red-400 transition-colors"
            >
              Abort
            </button>
          )}
          {state.phase === 'failed' && (
            <button
              onClick={() => {
                fetch(`/api/projects/${name}/retry`, { method: 'POST' }).catch(() => {});
              }}
              className="text-sm px-3 py-1 bg-neutral-700 hover:bg-neutral-600 text-neutral-200 rounded-lg transition-colors"
            >
              Retry
            </button>
          )}
        </div>
      </div>

      {/* Generic (non-POC) linear pipeline renders its own blocks. While a custom
          run is starting, the LinearProgress message hasn't arrived yet — show a
          placeholder instead of flashing the default POC stage list. */}
      {linear ? (
        <LinearPipeline projectName={name!} linear={linear} logs={logs} onApprove={() => approve()} onReject={reject} onExpand={setFocusedStage} />
      ) : state.custom_pipeline ? (
        <div className="flex items-center gap-2 text-neutral-500 text-base py-6">
          <span className="animate-pulse">●</span> Starting pipeline…
        </div>
      ) : (
      /* Stage Panels (POC pipeline) */
      <div className="space-y-0">
        {state.stages.map(stage => {
          // Flight Check is its own step after Tracks — so once tracks finish,
          // the Tracks panel reads as complete and review lives in its own panel.
          const inFlightCheck = state.phase === 'awaiting_code_review';
          const inTestFlight = state.phase === 'awaiting_test_plan' || state.phase === 'test_flying' || state.phase === 'awaiting_test_review';
          // Test Flight is opt-in: hide its row unless it's active or has run
          // (pending + never entered = user chose the land path, so don't show it).
          if (stage.id === 'test_flight' && stage.status === 'pending' && !inTestFlight) {
            return null;
          }
          const isActive = stage.status === 'running' ||
            (stage.id === 'plan' && state.phase === 'awaiting_plan_review') ||
            (stage.id === 'orbital_lanes' && (state.phase === 'running_tracks' || state.phase === 'setting_up_workspaces')) ||
            (stage.id === 'test_flight' && inTestFlight);

          return (
            <Fragment key={stage.id}>
            <StagePanel stage={stage} expanded={isActive || stage.status === 'failed'}>
              {stage.status === 'failed' && stage.error && (
                <div className="text-sm text-red-400 bg-red-950/30 rounded-lg p-3">
                  {stage.error}
                </div>
              )}

              {stage.status === 'running' && (stage.id === 'scan' || stage.id === 'plan') && (
                <div className="space-y-2">
                  <ActivityBar logCount={logs.length} />
                  <StreamingLog logs={logs} />
                  <button
                    onClick={() => setFocusedStage(stage.id)}
                    className="text-xs text-neutral-500 hover:text-neutral-300 transition-colors"
                  >
                    Expand ↗
                  </button>
                </div>
              )}

              {stage.id === 'plan' && state.phase === 'awaiting_plan_review' && (
                <div className="space-y-4">
                  <ReviewPanel
                    onApprove={approve}
                    onReject={reject}
                    content={planContent}
                  />
                  {state.tracks && state.tracks.length > 0 && (
                    <div className="text-sm text-neutral-500">
                      <span className="text-neutral-400">Tracks identified:</span>{' '}
                      {state.tracks.map(t => t.name || t.id).join(', ')}
                    </div>
                  )}
                </div>
              )}

              {stage.id === 'orbital_lanes' && state.phase === 'setting_up_workspaces' && (
                <div className="text-base text-neutral-400">
                  <span className="animate-pulse">●</span> Setting up workspaces...
                </div>
              )}

              {stage.id === 'orbital_lanes' && state.phase === 'running_tracks' && state.tracks && (
                <TrackProgressPanel projectName={name!} tracks={state.tracks} progress={trackProgress} />
              )}

              {stage.id === 'test_flight' && inTestFlight && (
                <TestFlightPanel
                  projectName={name!}
                  phase={state.phase}
                  logs={logs}
                  onApprove={approve}
                  onReject={reject}
                />
              )}

              {stage.id === 'land' && stage.status === 'running' && (
                <StreamingLog logs={logs} />
              )}
            </StagePanel>

            {/* Flight Check — rendered as its own step right after Tracks. */}
            {stage.id === 'orbital_lanes' && inFlightCheck && (
              <StagePanel
                stage={{ id: 'flight_check', status: 'running', loop_id: null, loop_pid: null, started_at: null, completed_at: null, error: null } as unknown as StageState}
                expanded
              >
                <FlightCheckPanel
                  projectName={name!}
                  onApprove={approve}
                  onReject={reject}
                />
              </StagePanel>
            )}
            </Fragment>
          );
        })}
      </div>
      )}

      {/* Feedback History */}
      {(state.plan_feedback_history.length > 0 || state.code_feedback_history.length > 0) && (
        <details className="mt-6 text-sm text-neutral-500">
          <summary className="cursor-pointer hover:text-neutral-400">
            Feedback history ({state.plan_feedback_history.length + state.code_feedback_history.length} rounds)
          </summary>
          <div className="mt-3 space-y-2 pl-3 border-l border-neutral-800">
            {state.plan_feedback_history.map((f, i) => (
              <div key={`plan-${i}`} className="text-neutral-500">
                <span className="text-neutral-400">Plan #{i + 1}:</span> {f}
              </div>
            ))}
            {state.code_feedback_history.map((f, i) => (
              <div key={`code-${i}`} className="text-neutral-500">
                <span className="text-neutral-400">Code #{i + 1}:</span> {f}
              </div>
            ))}
          </div>
        </details>
      )}

    </div>
  );
}
