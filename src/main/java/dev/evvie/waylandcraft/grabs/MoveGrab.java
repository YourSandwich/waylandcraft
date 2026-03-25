package dev.evvie.waylandcraft.grabs;

import dev.evvie.waylandcraft.WindowDisplay;
import dev.evvie.waylandcraft.WindowDisplay.DisplayHitResult;
import dev.evvie.waylandcraft.bridge.WLCAbstractWindow;
import dev.evvie.waylandcraft.bridge.WLCSurface;
import dev.evvie.waylandcraft.grabs.PointerGrabMap.MoveWorldEvent;
import dev.evvie.waylandcraft.grabs.PointerGrabMap.ReleasedImplicitGrab;
import net.minecraft.world.phys.Vec3;

public class MoveGrab extends PointerGrab {
	
	private final WindowDisplay window;
	private final MoveWorldEvent firstWorld;
	private Vec3 initialSurfaceLocal = null;
	
	public MoveGrab(ReleasedImplicitGrab implicit) {
		super(implicit.button());
		this.window = implicit.window();
		this.firstWorld = implicit.lastMoveEvent();
	}
	
	@Override
	public void init() throws GrabDroppedException {
		DisplayHitResult hitResult = window.intersect(firstWorld.pos(), firstWorld.view());
		if(hitResult == null) return;
		
		this.initialSurfaceLocal = hitResult.surfaceLocalOrigin;
	}
	
	@Override
	public void release() throws GrabDroppedException {
	}
	
	@Override
	public void moveWorld(Vec3 pos, Vec3 view, Vec3 up) throws GrabDroppedException {
		DisplayHitResult hitResult = window.intersect(pos, view);
		if(hitResult == null) return;
		
		Vec3 diff = hitResult.surfaceLocalOrigin.subtract(initialSurfaceLocal);
		window.pivot = window.pivot.add(window.localX().scale(diff.x).add(window.localY().scale(diff.y)));
	}
	
	@Override
	public void hover(WLCAbstractWindow window, WLCSurface surface, double x, double y) throws GrabDroppedException {
	}
	
}
