import { useState, useEffect, useCallback, useRef } from 'react';
import type { EngineState, WsServerMsg } from '../types/v2';

interface LogEntry {
  timestamp: string;
  level: string;
  message: string;
}

interface TrackProgress {
  track: string;
  tasks_done: number;
  tasks_total: number;
  current: string;
}

interface UseEngineReturn {
  state: EngineState | null;
  logs: LogEntry[];
  trackProgress: Record<string, TrackProgress>;
  linear: import('../types/v2').LinearProgress | null;
  connected: boolean;
  approve: (opts?: { testFlight?: boolean }) => void;
  reject: (feedback: string) => void;
  abort: () => void;
}

export function useEngine(projectName: string | undefined): UseEngineReturn {
  const [state, setState] = useState<EngineState | null>(null);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [trackProgress, setTrackProgress] = useState<Record<string, TrackProgress>>({});
  const [linear, setLinear] = useState<import('../types/v2').LinearProgress | null>(null);
  const [connected, setConnected] = useState(false);
  const wsRef = useRef<WebSocket | null>(null);

  useEffect(() => {
    if (!projectName) return;

    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const ws = new WebSocket(`${protocol}//${window.location.host}/ws/${projectName}`);
    wsRef.current = ws;

    ws.onopen = () => setConnected(true);
    ws.onclose = () => setConnected(false);
    ws.onerror = () => setConnected(false);

    ws.onmessage = (event) => {
      try {
        const msg: WsServerMsg = JSON.parse(event.data);
        switch (msg.type) {
          case 'state':
            setState(msg.data);
            break;
          case 'phase_change':
            setState(prev => prev ? { ...prev, phase: msg.phase } : prev);
            break;
          case 'log':
            setLogs(prev => [...prev.slice(-499), {
              timestamp: new Date().toISOString(),
              level: msg.level,
              message: msg.line,
            }]);
            break;
          case 'track_progress':
            setTrackProgress(prev => ({
              ...prev,
              [msg.track]: { track: msg.track, tasks_done: msg.tasks_done, tasks_total: msg.tasks_total, current: msg.current },
            }));
            break;
          case 'linear_progress':
            setLinear({
              template_id: msg.template_id,
              stages: msg.stages,
              current: msg.current,
              completed: msg.completed,
              paused: msg.paused,
              done: msg.done,
            });
            break;
        }
      } catch {}
    };

    return () => {
      ws.close();
      wsRef.current = null;
    };
  }, [projectName]);

  const approve = useCallback((opts?: { testFlight?: boolean }) => {
    fetch(`/api/projects/${projectName}/approve`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ test_flight: opts?.testFlight ?? false }),
    }).catch(() => {});
  }, [projectName]);

  const reject = useCallback((feedback: string) => {
    fetch(`/api/projects/${projectName}/reject`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ feedback }),
    }).catch(() => {});
  }, [projectName]);

  const abort = useCallback(() => {
    fetch(`/api/projects/${projectName}/abort`, { method: 'POST' }).catch(() => {});
  }, [projectName]);

  return { state, logs, trackProgress, linear, connected, approve, reject, abort };
}
