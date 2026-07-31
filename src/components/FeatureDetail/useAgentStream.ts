import { useEffect, useRef, useState } from 'react';
import { useTauriEvent } from '../../hooks/useTauriEvent';

export function useAgentStream(featureId: string) {
  const [streamContent, setStreamContent] = useState<Record<string, string>>({});
  const [activeStreamId, setActiveStreamId] = useState<string | null>(null);

  // Stream buffering: accumulate chunks in a ref, flush to state once per animation frame
  const streamBufferRef = useRef<Record<string, string>>({});
  const streamRafRef = useRef<number | null>(null);
  useEffect(() => () => {
    if (streamRafRef.current !== null) cancelAnimationFrame(streamRafRef.current);
  }, []);

  useTauriEvent<{ feature_id: string; step_execution_id: string; content: string }>('agent_stream', ({ feature_id, step_execution_id, content }) => {
    if (feature_id !== featureId) return;
    const buf = streamBufferRef.current;
    buf[step_execution_id] = (buf[step_execution_id] ?? '') + content;
    if (streamRafRef.current === null) {
      streamRafRef.current = requestAnimationFrame(() => {
        streamRafRef.current = null;
        setStreamContent({ ...streamBufferRef.current });
      });
    }
  });

  return { streamContent, activeStreamId, setActiveStreamId };
}
