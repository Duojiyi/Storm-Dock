import { createRoot } from "react-dom/client";
import "../../i18n";
import { mountStartupUpdateDialog } from "../../components/StartupUpdateDialog";
import { resolveHomeView } from "../../lib/homeTabs";
import { syncDocumentAppKind } from "../../lib/types";
import "../../styles/global.css";
import { HomePage } from "./HomePage";

syncDocumentAppKind(resolveHomeView().tab);
mountStartupUpdateDialog();
createRoot(document.getElementById("root")!).render(<HomePage />);
