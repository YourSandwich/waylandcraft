package dev.evvie.waylandcraft.bridge;

import org.jetbrains.annotations.Nullable;

import dev.evvie.waylandcraft.WaylandCraft;
import dev.evvie.waylandcraft.desktop.DesktopEntry;

public class WLCToplevel extends WLCAbstractWindow {

	@Nullable
	public String title;

	@Nullable
	public String appID;

	public ToplevelRequests requests = new ToplevelRequests();
	public boolean fullscreen = false;

	@Nullable
	public SurfaceGeometry restoreGeometry = null;

	public WLCToplevel(long handle) {
		super(handle);
	}

	// Best human-readable name for this window, or null if nothing is known.
	// Prefers the window title (Wayland toplevels and X11 windows both set it),
	// then the resolved desktop-entry name, then the raw app-id / X11 WM_CLASS.
	public @Nullable String displayName() {
		if(title != null && !title.isBlank()) return title;

		DesktopEntry entry = WaylandCraft.instance.xdgManager.forAppId(appID);
		if(entry != null && entry.name != null && !entry.name.isBlank()) return entry.name;

		if(appID != null && !appID.isBlank()) return appID;
		return null;
	}
	
	public static class ToplevelRequests {
		
		public boolean minimize = false;
		public boolean maximize = false;
		public boolean unmaximize = false;
		public boolean fullscreen = false;
		public boolean unfullscreen = false;
		
	}
	
}
