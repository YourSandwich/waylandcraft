package dev.evvie.waylandcraft.bridge;

import java.util.ArrayList;

import org.apache.commons.lang3.ArrayUtils;
import org.lwjgl.glfw.GLFW;
import org.lwjgl.glfw.GLFWNativeEGL;

import dev.evvie.waylandcraft.BufferTexture.DmabufTexture;
import net.minecraft.client.Minecraft;

public class WaylandCraftBridge {
	
	private long instance;
	private ArrayList<WLCToplevel> toplevels = new ArrayList<WLCToplevel>();
	private ArrayList<WLCSurface> surfaces = new ArrayList<WLCSurface>();
	private ArrayList<DmabufTexture> dmabufs = new ArrayList<DmabufTexture>();
	
	static {
		System.loadLibrary("waylandcraft");
	}
	
	private WaylandCraftBridge(long handle) {
		this.instance = handle;
	}
	
	public static WaylandCraftBridge start() {
		long eglDisplay = GLFWNativeEGL.glfwGetEGLDisplay();
		long eglConfig = GLFWNativeEGL.glfwGetEGLConfig(Minecraft.getInstance().getWindow().getWindow());
		
		if(eglDisplay == 0 || eglConfig == 0) {
			throw new RuntimeException("Failed to get EGL display or config!");
		}
		
		long handle = init(GLFW.Functions.GetProcAddress, eglDisplay);
		return new WaylandCraftBridge(handle);
	}
	
	protected WLCToplevel getOrCreateToplevel(long handle) {
		for(WLCToplevel toplevel : toplevels) {
			if(toplevel.getHandle() == handle) return toplevel;
		}
		WLCToplevel toplevel = new WLCToplevel(handle);
		
		long surfaceHandle = toplevelSurface(this.instance, handle);
		WLCSurface surface = getOrCreateSurface(surfaceHandle);
		toplevel.surface = surface;
		
		toplevels.add(toplevel);
		return toplevel;
	}
	
	protected WLCSurface getOrCreateSurface(long handle) {
		for(WLCSurface surface : surfaces) {
			if(surface.getHandle() == handle) return surface;
		}
		WLCSurface surface = new WLCSurface(handle);
		surfaces.add(surface);
		return surface;
	}
	
	protected DmabufTexture getDmabuf(long handle) {
		for(DmabufTexture dmabuf : dmabufs) {
			if(dmabuf.handle == handle) return dmabuf;
		}
		return null;
	}
	
	protected void addDmabuf(DmabufTexture dmabuf) {
		dmabufs.add(dmabuf);
	}
	
	private void deleteNonExistingToplevels(long[] remainingHandles) {
		ArrayList<WLCToplevel> toplevels_new = new ArrayList<WLCToplevel>();
		for(WLCToplevel toplevel : this.toplevels) {
			if(ArrayUtils.contains(remainingHandles, toplevel.getHandle())) {
				toplevels_new.add(toplevel);
			}
			else {
				freeToplevel(this.instance, toplevel.takeHandle());
			}
		}
		this.toplevels = toplevels_new;
	}
	
	private void deleteNonExistingDmabufs(long[] remainingHandles) {
		ArrayList<DmabufTexture> dmabufs_new = new ArrayList<DmabufTexture>();
		for(DmabufTexture dmabuf : this.dmabufs) {
			if(ArrayUtils.contains(remainingHandles, dmabuf.handle)) {
				dmabufs_new.add(dmabuf);
			}
			else {
				dmabuf.free();
			}
		}
		this.dmabufs = dmabufs_new;
	}
	
	private void deleteUnvisitedSurfaces() {
		ArrayList<WLCSurface> surfaces_new = new ArrayList<WLCSurface>();
		for(WLCSurface surface : this.surfaces) {
			if(surface.visited) {
				surfaces_new.add(surface);
			}
			else {
				freeSurface(this.instance, surface.takeHandle());
			}
		}
		this.surfaces = surfaces_new;
	}
	
	public void update() {
		// Update wayland clients
		update(this.instance);
		
		// Find all available toplevels and delete ones that no longer exist
		long[] toplevel_handles = toplevels(instance);
		deleteNonExistingToplevels(toplevel_handles);
		
		// Reset surface visited state
		for(WLCSurface surface : surfaces) {
			surface.visited = false;
		}
		
		// Create new toplevels when necessary
		// Update surface tree geometry of all toplevels
		for(long handle : toplevel_handles) {
			WLCToplevel toplevel = getOrCreateToplevel(handle);
			WLCSurface root = toplevel.getSurfaceTree();
			toplevel.lastChild = updateSurfaceTree(root);
		}
		
		// All surface trees have now been walked. Now delete all unvisited surfaces
		deleteUnvisitedSurfaces();
		
		// Resolve surface parent handles to actual surfaces
		for(WLCSurface surface : surfaces) {
			if(surface.parentHandle != 0) {
				surface.parent = getOrCreateSurface(surface.parentHandle);
			}
			else {
				surface.parent = null;
			}
		}
		
		// Update all surface buffers
		for(WLCToplevel toplevel : toplevels) {
			WLCSurface root = toplevel.getSurfaceTree();
			for(WLCSurface surface = root; surface != null; surface = surface.getNextChild()) {
				updateSurfaceData(instance, surface);
				calculateSubpos(surface);
			}
		}
		
		deleteNonExistingDmabufs(dmabufs(instance));
		
		// Do client frame callbacks
		sendFrame(instance);
	}
	
	private void calculateSubpos(WLCSurface surface) {
		if(surface.parent != null) {
			calculateSubpos(surface.parent);
			surface.xSubpos = surface.parent.xSubpos + surface.xoff;
			surface.ySubpos = surface.parent.ySubpos + surface.yoff;
		}
		else {
			surface.xSubpos = 0;
			surface.ySubpos = 0;
		}
	}
	
	public WLCToplevel[] getToplevels() {
		return toplevels.toArray(new WLCToplevel[toplevels.size()]);
	}
	
	public String getSocket() {
		return socket(this.instance);
	}
	
	public boolean inputRegionContains(WLCSurface surface, double x, double y) {
		return checkInputRegion(surface.getHandle(), x, y);
	}
	
	public void sendMotion(WLCSurface surface, double x, double y) {
		surfaceMotion(instance, surface.getHandle(), x, y);
	}
	
	public void sendMotionOutside() {
		long handle = 0;
		surfaceMotion(instance, handle, 0.0, 0.0);
	}
	
	public void sendButton(int button, int state) {
		pointerButton(instance, button, state);
	}
	
	private static native long init(long glfwGetProcAddress, long eglDisplay);
	private static native void update(long instance);
	private static native String socket(long instance);
	private static native void sendFrame(long instance);
	
	private static native long[] toplevels(long instance);
	private static native long toplevelSurface(long instance, long handle);
	private static native void updateSurfaceData(long instance, WLCSurface surface);
	
	private static native long[] dmabufs(long instance);
	
	// Updates the surface tree given by the root surface
	// This changes the doubly linked list of the WLCSurfaces.
	// The returned surface is the last (most deeply nested) child
	private native WLCSurface updateSurfaceTree(WLCSurface root);
	
	// Check if point in surface input region
	private static native boolean checkInputRegion(long surfaceHandle, double x, double y);
	
	// Create pointer motion event, handle can be 0 when under no surface
	private static native void surfaceMotion(long instance, long handle, double x, double y);
	
	// Create pointer button event. `button` has to be the linux button code, state is 1 for pressed, 0 for released
	private static native void pointerButton(long instance, int button, int state);
	
	private static native void freeSurface(long instance, long handle);
	private static native void freeToplevel(long instance, long handle);
	
}
