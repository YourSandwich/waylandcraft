package dev.evvie.waylandcraft.grabs;

import dev.evvie.waylandcraft.WaylandCraftUtils;
import dev.evvie.waylandcraft.WindowDisplay;
import dev.evvie.waylandcraft.bridge.WLCToplevel;
import net.minecraft.world.phys.Vec3;

public class WindowGrab extends PointerGrab {

	private final WindowDisplay window;
	private final WindowResize resize;

	public WindowGrab(WindowDisplay window, int button) {
		super(button);
		this.window = window;
		this.resize = new WindowResize((WLCToplevel) window.window);
		window.anchorDistance = 2.0;
		window.anchorHeight = 0.0;
		window.anchorLateral = 0.0;
	}

	private void checkValid() throws GrabDroppedException {
		if(!window.isValid()) {
			this.drop();
		}
	}

	@Override
	public void init() throws GrabDroppedException {
		this.checkValid();
	}

	@Override
	public void release(boolean force) throws GrabDroppedException {
		this.resize.commit();
		this.checkValid();
	}

	@Override
	public void moveWorld(Vec3 pos, Vec3 view, Vec3 up) throws GrabDroppedException {
		this.checkValid();

		// A resize drag holds the window still so the view sweep can size it;
		// finalize the resize once Control is released. Re-anchoring otherwise
		// keeps Shift+scroll lateral movement responsive.
		if(resize.isResizing()) {
			if(WaylandCraftUtils.isControlHeld()) return;
			this.resize.commit();
		}

		window.anchorToPosView(pos, view, up);
	}

	@Override
	public void onScroll(double scrollX, double scrollY) throws GrabDroppedException {
		this.checkValid();

		if(WaylandCraftUtils.isAltHeld()) window.adjustAnchorHeight(scrollY);
		else if(WaylandCraftUtils.isShiftHeld()) window.adjustAnchorLateral(scrollY);
		else window.adjustAnchorDistance(scrollY);
	}

	@Override
	public boolean onMouseTurn(double dx, double dy) throws GrabDroppedException {
		this.checkValid();

		if(!WaylandCraftUtils.isControlHeld()) return false;

		this.resize.onMouseTurn(dx, dy);
		return true;
	}

}
