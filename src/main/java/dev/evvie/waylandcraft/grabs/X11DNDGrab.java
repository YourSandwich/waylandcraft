package dev.evvie.waylandcraft.grabs;

import dev.evvie.waylandcraft.bridge.WLCAbstractWindow;
import dev.evvie.waylandcraft.bridge.WLCSurface;
import net.minecraft.world.phys.Vec3;

/* Pointer grab for an X11-initiated drag (Stage C of X11<->Wayland DnD).
 *
 * Unlike DNDGrab, this is not started from a Wayland start_drag - an X11 app
 * began the drag and owns the X pointer grab. WaylandCraft only needs the
 * in-world cursor ray-cast to pick the Wayland surface under it and feed
 * ddm::dnd_motion. Drop and cancel are decided by the X11 source's XDND
 * messages, not the in-world mouse, so this grab forwards neither.
 *
 * The grab is started and ended by WaylandCraft polling bridge.isX11DndActive():
 * it carries no real mouse button (button -1 never matches a button release),
 * so it is only ever ended by releaseAll() once the X11 drag finishes.
 */
public class X11DNDGrab extends PointerGrab {

	public X11DNDGrab() {
		super(-1);
	}

	@Override
	public void init() throws GrabDroppedException {
		wlc.bridge.sendMotionOutside();
	}

	@Override
	public void release(boolean force) throws GrabDroppedException {
		// No-op: the X11 drag's lifecycle belongs to the X11 source (its XDND
		// drop/leave) and the XdndSelection, never the in-world mouse. This
		// grab is only a motion driver; releasing it just stops driving
		// motion. The drag itself ends natively, which the poll then observes.
	}

	@Override
	public void moveWorld(Vec3 pos, Vec3 view, Vec3 up) throws GrabDroppedException {
	}

	@Override
	public void hover(WLCAbstractWindow window, WLCSurface surface, double x, double y) throws GrabDroppedException {
		wlc.bridge.sendDndMotion(surface, x, y);
	}

	@Override
	public void hoverNone() throws GrabDroppedException {
		wlc.bridge.sendDndMotion(null, 0, 0);
	}

}
