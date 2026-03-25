package dev.evvie.waylandcraft.grabs;

import java.util.ArrayList;

import org.jetbrains.annotations.Nullable;

import dev.evvie.waylandcraft.WaylandCraft;
import dev.evvie.waylandcraft.WindowDisplay;
import dev.evvie.waylandcraft.WindowDisplay.DisplayHitResult;
import dev.evvie.waylandcraft.bridge.WLCAbstractWindow;
import dev.evvie.waylandcraft.bridge.WLCSurface;
import net.minecraft.world.phys.Vec3;

public class PointerGrabMap {
	
	private WaylandCraft wlc;
	private PointerGrab exclusiveGrab = null;
	private ImplicitGrabs implicitGrabs = null;
	
	public PointerGrabMap(WaylandCraft wlc) {
		this.wlc = wlc;
	}
	
	public boolean isGrabActive() {
		return implicitGrabs != null || exclusiveGrab != null;
	}
	
	public boolean isExclusiveGrabActive() {
		return exclusiveGrab != null;
	}
	
	public boolean isGrabActive(int button) {
		return (exclusiveGrab != null && exclusiveGrab.button == button) || (implicitGrabs != null && implicitGrabs.contains(button));
	}
	
	// Start implicit pointer grab on a surface. Surface MUST have active pointer focus!
	public void startImplicit(WindowDisplay window, WLCSurface surface, int button) {
		if(isExclusiveGrabActive()) return;
		
		if(implicitGrabs == null) implicitGrabs = new ImplicitGrabs(window, surface);
		if(implicitGrabs.contains(button)) return;
		
		int serial = wlc.bridge.sendButton(0x110 + button, 1);
		implicitGrabs.add(window, surface, button, serial);
	}
	
	public void startExclusive(PointerGrab grab) {
		if(isExclusiveGrabActive()) return;
		
		this.releaseImplicit();
		
		try {
			grab.init();
		} catch (GrabDroppedException e) {
			return;
		}
		
		exclusiveGrab = grab;
	}
	
	public void moveWorld(Vec3 pos, Vec3 view, Vec3 up) {
		if(exclusiveGrab != null) {
			try {
				exclusiveGrab.moveWorld(pos, view, up);
			} catch(GrabDroppedException e) {
				exclusiveGrab = null;
			}
			
			return;
		}
		
		if(implicitGrabs == null) return;
		
		implicitGrabs.updateMoveWorld(pos, view, up);
		
		DisplayHitResult hitResult = implicitGrabs.window.intersect(pos, view);
		if(hitResult == null) return;
		
		Vec3 relativeCoords = hitResult.surfaceLocalOrigin.subtract(implicitGrabs.surface.xSubpos, implicitGrabs.surface.ySubpos, 0);
		wlc.bridge.sendMotion(relativeCoords.x, relativeCoords.y);
	}
	
	public void hover(WLCAbstractWindow window, WLCSurface surface, double x, double y) {
		if(exclusiveGrab != null) {
			try {
				exclusiveGrab.hover(window, surface, x, y);
			} catch(GrabDroppedException e) {
				exclusiveGrab = null;
			}
		}
	}
	
	public void release(int button) {
		if(exclusiveGrab != null && exclusiveGrab.button == button) {
			try {
				exclusiveGrab.release();
			} catch (GrabDroppedException e) {
				// No handling necessary, grab always removed
			}
			exclusiveGrab = null;
			return;
		}
		
		if(implicitGrabs == null) return;
		
		if(implicitGrabs.contains(button)) {
			wlc.bridge.sendButton(0x110 + button, 0);
			implicitGrabs.remove(button);
		}
		
		if(implicitGrabs.isEmpty()) implicitGrabs = null;
	}
	
	private void releaseImplicit() {
		if(implicitGrabs == null) return;
		
		for(ImplicitGrab press : implicitGrabs.entries) {
			wlc.bridge.sendButton(0x110 + press.button, 0);
		}
		implicitGrabs = null;
	}
	
	public void releaseAll() {
		this.releaseImplicit();
		
		if(exclusiveGrab == null) return;
		
		try {
			exclusiveGrab.release();
		} catch (GrabDroppedException e) {
			// No handling necessary, grab always removed
		}
		exclusiveGrab = null;
	}
	
	public @Nullable ReleasedImplicitGrab releaseImplicitMatching(int serial) {
		if(isExclusiveGrabActive()) return null;
		if(implicitGrabs == null) return null;
		if(implicitGrabs.lastMoveEvent == null) return null;
		
		for(ImplicitGrab implicit : implicitGrabs.entries) {
			if(implicit.serial == serial) {
				MoveWorldEvent lastMoveEvent = implicitGrabs.lastMoveEvent;
				release(implicit.button); // Warning: Could set implicitGrabs to null
				return new ReleasedImplicitGrab(implicit, lastMoveEvent);
			}
		}
		return null;
	}
	
	private static class ImplicitGrabs {
		
		public final WindowDisplay window;
		public final WLCSurface surface;
		public ArrayList<ImplicitGrab> entries = new ArrayList<ImplicitGrab>();
		public MoveWorldEvent lastMoveEvent = null;
		
		public ImplicitGrabs(WindowDisplay window, WLCSurface surface) {
			this.window = window;
			this.surface = surface;
		}
		
		public boolean contains(int button) {
			return entries.stream().anyMatch((press) -> press.button == button);
		}
		
		public boolean isEmpty() {
			return entries.isEmpty();
		}
		
		public void add(WindowDisplay display, WLCSurface surface, int button, int serial) {
			assert !contains(button);
			entries.add(new ImplicitGrab(display, surface, button, serial));
		}
		
		public void remove(int button) {
			assert contains(button);
			entries.removeIf((press) -> press.button == button);
		}
		
		public void updateMoveWorld(Vec3 pos, Vec3 view, Vec3 up) {
			this.lastMoveEvent = new MoveWorldEvent(pos, view, up);
		}
		
	}
	
	// Not a real pointer grab, just a way to represent active button presses on a WindowDisplay
	private static record ImplicitGrab(WindowDisplay window, WLCSurface surface, int button, int serial) {}
	public static record MoveWorldEvent(Vec3 pos, Vec3 view, Vec3 up) {}
	public static record ReleasedImplicitGrab(WindowDisplay window, WLCSurface surface, MoveWorldEvent lastMoveEvent, int button, int serial) {
		protected ReleasedImplicitGrab(ImplicitGrab implicitGrab, MoveWorldEvent lastMoveEvent) {
			this(implicitGrab.window(), implicitGrab.surface(), lastMoveEvent, implicitGrab.button(), implicitGrab.serial());
		}
	}
	
}
