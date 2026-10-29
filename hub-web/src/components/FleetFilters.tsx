import { Search } from 'lucide-react';
import { useId } from 'react';

import { SegmentedControl } from '../../../src/components/ui/SegmentedControl';
import type { FleetFilter } from '../lib/fleetSummary';

interface FleetFiltersProps {
  filter: FleetFilter;
  counts: Record<FleetFilter, number>;
  query: string;
  onFilter: (filter: FleetFilter) => void;
  onQuery: (query: string) => void;
}

export function FleetFilters({ filter, counts, query, onFilter, onQuery }: FleetFiltersProps) {
  const searchId = useId();
  const options = [
    { value: 'all', label: 'All', count: counts.all },
    { value: 'online', label: 'Online', count: counts.online },
    { value: 'offline', label: 'Offline', count: counts.offline },
    { value: 'attention', label: 'Attention', count: counts.attention, countTone: 'amber' },
  ] as const;
  return (
    <div className="flex flex-wrap items-center gap-3">
      <SegmentedControl options={options} value={filter} onChange={onFilter} ariaLabel="Status" />
      <div className="relative ml-auto w-full max-w-sm sm:w-auto sm:flex-[1_1_240px]">
        <label htmlFor={searchId} className="sr-only">
          Search instances
        </label>
        <Search
          aria-hidden="true"
          className="pointer-events-none absolute top-1/2 left-3 h-4 w-4 -translate-y-1/2 text-slate-400"
        />
        <input
          id={searchId}
          type="search"
          value={query}
          onChange={(event) => onQuery(event.target.value)}
          placeholder="Search instances, projects, harnesses"
          className="input-field pl-9"
        />
      </div>
    </div>
  );
}
