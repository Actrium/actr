import { useEffect, useState } from "react";
import { NavLink } from "react-router-dom";
import { useAuth } from "../../lib/auth";
import {
  LayoutDashboard,
  Globe,
  FileText,
  LogOut,
  ChevronsLeft,
  ChevronsRight,
  Shield,
  ArrowLeftRight,
  Radio,
  Fingerprint,
  Key,
  Activity,
  MonitorDot,
  Network,
  BookOpen,
  LifeBuoy,
  Info,
  Building2,
  type LucideIcon,
} from "lucide-react";
import { cn } from "../../lib/utils";
import { api } from "../../lib/api";

const SIDEBAR_COLLAPSED_KEY = "actrix_admin_sidebar_collapsed";

interface NavItem {
  to: string;
  icon: LucideIcon;
  label: string;
  end?: boolean;
}

const navItems: NavItem[] = [
  { to: "/admin", icon: LayoutDashboard, label: "Dashboard", end: true },
  { to: "/admin/general/architecture", icon: Network, label: "Architecture" },
  { to: "/admin/config", icon: FileText, label: "Config", end: false },
  { to: "/admin/realms", icon: Globe, label: "Realms", end: false },
  { to: "/admin/general/recording", icon: Activity, label: "Recording" },
];

const serviceItems: NavItem[] = [
  { to: "/admin/services/stun", icon: Shield, label: "STUN" },
  { to: "/admin/services/turn", icon: ArrowLeftRight, label: "TURN" },
  { to: "/admin/services/signaling", icon: Radio, label: "Signaling" },
  { to: "/admin/mfr", icon: Building2, label: "MFR" },
  { to: "/admin/services/ais", icon: Fingerprint, label: "AIS" },
  { to: "/admin/services/signer", icon: Key, label: "Signer" },
];

const hostItems: NavItem[] = [
  { to: "/admin/host/status", icon: MonitorDot, label: "Status" },
  { to: "/admin/host/network", icon: Network, label: "Network" },
];

const resourceItems: NavItem[] = [
  { to: "https://github.com/Actrium/actrix", icon: BookOpen, label: "Documentation" },
  { to: "https://github.com/Actrium/actrix/releases", icon: BookOpen, label: "Releases" },
  { to: "https://github.com/Actrium/actrix/discussions", icon: LifeBuoy, label: "Community" },
  { to: "/admin/about", icon: Info, label: "About" },
];

function readCollapsedFromStorage(): boolean {
  if (typeof window === "undefined") {
    return false;
  }
  return window.localStorage.getItem(SIDEBAR_COLLAPSED_KEY) === "1";
}

function extractNodeName(
  fields: Array<{ key: string; effective_value: string }>,
): string {
  const nameField = fields.find((field) => field.key === "name");
  const name = nameField?.effective_value.trim();
  if (!name || name === "—") {
    return "Unnamed node";
  }
  return name;
}

interface NavItemsProps {
  items: NavItem[];
  collapsed: boolean;
}

interface SectionTitleProps {
  label: string;
  collapsed: boolean;
}

function NavItems({ items, collapsed }: NavItemsProps) {
  return (
    <>
      {items.map(({ to, icon: Icon, label, end }) => (
        to.startsWith("http") ? (
          <a
            key={to}
            href={to}
            target="_blank"
            rel="noreferrer"
            title={collapsed ? label : undefined}
            className={cn(
              "flex h-9 items-center rounded-lg px-3 text-sm font-medium transition-colors",
              collapsed ? "justify-center" : "gap-3",
              "text-gray-600 hover:bg-gray-100 hover:text-gray-900",
            )}
          >
            <Icon className="h-4 w-4 shrink-0" />
            {!collapsed && <span className="truncate">{label}</span>}
          </a>
        ) : (
          <NavLink
            key={to}
            to={to}
            end={end}
            title={collapsed ? label : undefined}
            className={({ isActive }) =>
              cn(
                "flex h-9 items-center rounded-lg px-3 text-sm font-medium transition-colors",
                collapsed ? "justify-center" : "gap-3",
                isActive
                  ? "bg-blue-50 text-blue-700"
                  : "text-gray-600 hover:bg-gray-100 hover:text-gray-900",
              )
            }
          >
            <Icon className="h-4 w-4 shrink-0" />
            {!collapsed && <span className="truncate">{label}</span>}
          </NavLink>
        )
      ))}
    </>
  );
}

function SectionTitle({ label, collapsed }: SectionTitleProps) {
  return (
    <div className="flex h-9 items-end px-3">
      <p
        aria-hidden={collapsed}
        className={cn(
          "text-xs font-semibold uppercase tracking-wider leading-4",
          collapsed ? "invisible" : "text-gray-400",
        )}
      >
        {label}
      </p>
    </div>
  );
}

export function Sidebar() {
  const { logout } = useAuth();
  const [collapsed, setCollapsed] = useState<boolean>(() =>
    readCollapsedFromStorage(),
  );
  const [nodeName, setNodeName] = useState("Loading...");

  useEffect(() => {
    window.localStorage.setItem(SIDEBAR_COLLAPSED_KEY, collapsed ? "1" : "0");
  }, [collapsed]);

  useEffect(() => {
    let active = true;

    const loadNodeName = async () => {
      try {
        const data = await api.getPlatformDetail();
        if (active) {
          setNodeName(extractNodeName(data.config_fields));
        }
      } catch {
        if (active) {
          setNodeName("Unknown node");
        }
      }
    };

    void loadNodeName();
    return () => {
      active = false;
    };
  }, []);

  return (
    <aside
      className={cn(
        "sticky top-0 flex h-screen shrink-0 self-start flex-col border-r border-gray-200 bg-white transition-[width] duration-200",
        collapsed ? "w-16" : "w-56",
      )}
    >
      <div
        className={cn(
          "border-b border-gray-200",
          collapsed ? "px-2 py-3" : "px-4 py-3",
        )}
      >
        <div className="flex items-start justify-between gap-1.5">
          <div className="min-w-0">
            <div className="flex h-7 items-baseline gap-1.5">
              {collapsed ? (
                <span className="relative -top-px rounded bg-blue-100 px-[4px] py-px text-lg font-semibold text-blue-600">
                  A
                </span>
              ) : (
                <span className="text-lg font-bold tracking-tight text-gray-900">
                  Actrix
                </span>
              )}
              {!collapsed && (
                <span className="relative -top-px rounded bg-blue-100 px-[4px] py-px text-sm font-semibold text-blue-600">
                  Admin
                </span>
              )}
            </div>
            <p
              className={cn(
                "mt-1 h-4 truncate text-xs leading-4 font-medium text-gray-500",
                collapsed ? "text-center" : "",
              )}
              title={nodeName}
            >
              {nodeName}
            </p>
          </div>
          <button
            type="button"
            onClick={() => setCollapsed((v) => !v)}
            className="rounded-md p-1 text-gray-500 transition-colors hover:bg-gray-100 hover:text-gray-900"
            aria-label={collapsed ? "Expand sidebar menu" : "Collapse sidebar menu"}
            title={collapsed ? "Expand menu" : "Collapse menu"}
          >
            {collapsed ? (
              <ChevronsRight className="h-4 w-4" />
            ) : (
              <ChevronsLeft className="h-4 w-4" />
            )}
          </button>
        </div>
      </div>

      <nav className={cn("min-h-0 flex-1 space-y-1 overflow-y-auto", collapsed ? "p-2" : "p-3")}>
        <NavItems items={navItems} collapsed={collapsed} />

        {/* Services section */}
        <SectionTitle label="Services" collapsed={collapsed} />
        <NavItems items={serviceItems} collapsed={collapsed} />

        {/* Host section */}
        <SectionTitle label="Host" collapsed={collapsed} />
        <NavItems items={hostItems} collapsed={collapsed} />

        {/* Help / Resources section */}
        <SectionTitle label="Help / Resources" collapsed={collapsed} />
        <NavItems items={resourceItems} collapsed={collapsed} />
      </nav>

      <div className={cn("border-t border-gray-200", collapsed ? "p-2" : "p-3")}>
        <button
          onClick={logout}
          title={collapsed ? "Logout" : undefined}
          className={cn(
            "flex w-full items-center rounded-lg px-3 py-2 text-sm font-medium text-gray-600 transition-colors hover:bg-gray-100 hover:text-gray-900",
            collapsed ? "justify-center" : "gap-3",
          )}
        >
          <LogOut className="h-4 w-4" />
          {!collapsed && "Logout"}
        </button>
      </div>
    </aside>
  );
}
