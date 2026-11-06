import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { errorReportRecord } from "./lib/bridge";
import { installMenuGuard } from "./lib/contextMenu";
import { installErrorReporting } from "./lib/errorReport";

// No « Reload » in a right-click, none by Cmd+R / Ctrl+R / F5: a reload loses
// the wizard's step and choices.
installMenuGuard(window);

// An uncaught error of the window goes to the core, which reports it only
// where the person's setting and the service allow it.
installErrorReporting(window, errorReportRecord);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
