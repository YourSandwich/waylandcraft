package dev.evvie.waylandcraft.grabs;

import dev.evvie.waylandcraft.WaylandCraft;
import dev.evvie.waylandcraft.bridge.WLCToplevel;
import dev.evvie.waylandcraft.bridge.WaylandCraftBridge.Size;

/* Shared Control+drag resize for a window being placed, used by both the
 * window-manager grab and the place-from-hand path so the resize behaves
 * identically. Holds the pending size while the drag is in progress.
 */
public class WindowResize {

	private final WLCToplevel toplevel;

	// Pending resize size while Control is held, or -1 when not resizing
	private int width = -1;
	private int height = -1;

	public WindowResize(WLCToplevel toplevel) {
		this.toplevel = toplevel;
	}

	// True while a resize drag is in progress and the window should stay put
	public boolean isResizing() {
		return width >= 0;
	}

	// Apply a view-drag delta to the size, same math as the window manager screen
	public void onMouseTurn(double dx, double dy) {
		if(width < 0) {
			width = toplevel.geometry.width();
			height = toplevel.geometry.height();
		}

		Size bounds = WaylandCraft.instance.bridge.getOutputBounds();
		width = Math.clamp(width + (int) dx / 2, 0, bounds.width());
		height = Math.clamp(height + (int) dy / 2, 0, bounds.height());

		WaylandCraft.instance.bridge.resizeToplevelInteractive(toplevel, width, height);
	}

	// Finish an in-progress resize, clearing the toplevel's interactive resize state
	public void commit() {
		if(width < 0) return;

		if(toplevel.isAlive()) WaylandCraft.instance.bridge.resizeToplevel(toplevel, width, height);
		width = height = -1;
	}

}
