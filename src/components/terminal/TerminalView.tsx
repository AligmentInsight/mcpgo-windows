// Terminal view - the Phase 3 face of the PowerShell job engine.
// Composition only: data flows through the useJobs / useJobLog hooks, the
// presentation lives in CommandForm, JobListPanel and JobOutputPane.

import CommandForm from "./CommandForm";
import JobListPanel from "./JobListPanel";
import JobOutputPane from "./JobOutputPane";
import { useJobLog } from "../../hooks/useJobLog";
import { useJobs } from "../../hooks/useJobs";

export default function TerminalView() {
  const { jobs, selectedId, select, start, cancel, starting, startError } = useJobs();
  const { lines, loading } = useJobLog(selectedId);
  const selectedJob = jobs.find((job) => job.id === selectedId) ?? null;

  return (
    <div className="space-y-4">
      <CommandForm
        onRun={(command, workingDir) => {
          void start(command, workingDir);
        }}
        running={starting}
        error={startError}
      />
      <div className="grid items-start gap-4 lg:grid-cols-[300px_minmax(0,1fr)]">
        <JobListPanel
          jobs={jobs}
          selectedId={selectedId}
          onSelect={select}
          onCancel={(jobId) => {
            void cancel(jobId);
          }}
        />
        <JobOutputPane
          job={selectedJob}
          lines={lines}
          loading={loading}
          onCancel={(jobId) => {
            void cancel(jobId);
          }}
        />
      </div>
    </div>
  );
}
