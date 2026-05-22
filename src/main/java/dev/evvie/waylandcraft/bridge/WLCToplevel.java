package dev.evvie.waylandcraft.bridge;

import java.util.Arrays;

import org.jetbrains.annotations.Nullable;

import dev.evvie.waylandcraft.WaylandCraft;
import dev.evvie.waylandcraft.desktop.DesktopEntry;
import dev.evvie.waylandcraft.desktop.DesktopIcon;
import net.minecraft.resources.Identifier;

public class WLCToplevel extends WLCAbstractWindow {

	// X11 apps have no .desktop entry; their icon comes from _NET_WM_ICON, which
	// the bridge polls. A missing property is retried sooner than a present one
	// is refreshed - most X11 windows publish an icon shortly after mapping.
	private static final long ICON_RETRY_MS = 1000L;
	private static final long ICON_REFRESH_MS = 5000L;

	@Nullable
	public String title;

	@Nullable
	public String appID;

	public ToplevelRequests requests = new ToplevelRequests();
	public boolean fullscreen = false;

	@Nullable
	public SurfaceGeometry restoreGeometry = null;

	// _NET_WM_ICON state for X11 windows; stays null for Wayland toplevels.
	@Nullable
	private DesktopIcon x11Icon = null;
	private int x11IconHash = 0;
	private long nextIconFetchMs = 0;

	public WLCToplevel(long handle) {
		super(handle);
	}

	// Texture for this window's icon: an X11 window's own _NET_WM_ICON when it
	// has one, otherwise the resolved .desktop entry icon. Null if neither.
	public @Nullable Identifier iconTexture() {
		if(x11Icon != null) return x11Icon.getTextureLocation();

		DesktopEntry entry = WaylandCraft.instance.xdgManager.forAppId(appID);
		if(entry == null) return null;
		return entry.getIcon();
	}

	// True when the bridge should poll _NET_WM_ICON for this window again.
	public boolean shouldFetchIcon(long nowMs) {
		return nowMs >= nextIconFetchMs;
	}

	// Feed in a freshly read _NET_WM_ICON: [width, height, then width*height
	// ARGB pixels], or null when the window publishes no icon. Rebuilds the
	// cached icon only when the pixel data actually changed.
	public void updateWindowIcon(@Nullable int[] iconData, long nowMs) {
		if(iconData == null || iconData.length < 3) {
			nextIconFetchMs = nowMs + ICON_RETRY_MS;
			return;
		}

		int width = iconData[0];
		int height = iconData[1];
		if(width <= 0 || height <= 0 || iconData.length != (long) width * height + 2) {
			nextIconFetchMs = nowMs + ICON_RETRY_MS;
			return;
		}

		nextIconFetchMs = nowMs + ICON_REFRESH_MS;

		int hash = Arrays.hashCode(iconData);
		if(x11Icon != null && x11IconHash == hash) return;

		int[] argb = Arrays.copyOfRange(iconData, 2, iconData.length);
		x11Icon = DesktopIcon.fromArgb(Long.toHexString(getHandle()) + "_" + hash, width, height, argb);
		x11IconHash = hash;
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
