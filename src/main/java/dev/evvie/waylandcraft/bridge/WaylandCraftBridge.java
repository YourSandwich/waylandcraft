package dev.evvie.waylandcraft.bridge;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.LinkedList;
import java.util.List;
import java.util.Set;
import java.util.stream.Collectors;
import java.util.stream.Stream;

import org.apache.commons.lang3.ArrayUtils;
import org.jetbrains.annotations.Nullable;
import org.lwjgl.glfw.GLFW;
import org.lwjgl.glfw.GLFWNativeEGL;

import dev.evvie.waylandcraft.CursorShape;
import dev.evvie.waylandcraft.WaylandCraft;
import dev.evvie.waylandcraft.bridge.WLCAbstractWindow.SurfaceGeometry;
import dev.evvie.waylandcraft.desktop.RawDesktopEntry;
import dev.evvie.waylandcraft.render.BufferTexture.DmabufTexture;
import dev.evvie.waylandcraft.render.WindowFramebuffer;
import net.minecraft.util.profiling.Profiler;
import net.minecraft.util.profiling.ProfilerFiller;

public class WaylandCraftBridge {
	
	private long instance;
	private ArrayList<WLCToplevel> toplevels = new ArrayList<WLCToplevel>();
	private ArrayList<WLCPopup> popups = new ArrayList<WLCPopup>();
	private ArrayList<WLCSurface> surfaces = new ArrayList<WLCSurface>();
	private ArrayList<DmabufTexture> dmabufs = new ArrayList<DmabufTexture>();
	private ArrayList<WindowFramebuffer> framebuffers = new ArrayList<WindowFramebuffer>();
	
	public IconSurface dndIcon = null;
	
	private LinkedList<WLCToplevel> focusOrder = new LinkedList<WLCToplevel>();
	
	private ArrayList<WLCToplevel> newToplevels = new ArrayList<WLCToplevel>();
	
	private @Nullable Integer lastMoveRequestSerial = null;
	private @Nullable ResizeRequest lastResizeRequest = null;

	// Shared handle-0 surface for windows with no backing surface (an X11 window
	// before the xwayland-shell association and while unmapped). It is never put
	// in the surfaces list - it has no native pointer to free - and is inert:
	// not alive, no buffer, zero size, no children.
	private static final WLCSurface NO_SURFACE = new WLCSurface(0);
	
	static {
		boolean loaded = false;
		InputStream inputStream = WaylandCraftBridge.class.getResourceAsStream("/libwaylandcraft.so");
		if(inputStream != null) {
			try {
				byte[] data = inputStream.readAllBytes();
				inputStream.close();
				
				File temp = File.createTempFile("waylandcraft-", "-libwaylandcraft.so");
				temp.deleteOnExit();
				
				FileOutputStream outputStream = new FileOutputStream(temp);
				outputStream.write(data);
				outputStream.close();
				
				System.load(temp.getAbsolutePath());
				loaded = true;
				
				WaylandCraft.LOGGER.info("Loaded native library from jar");
			} catch (IOException e) {
				e.printStackTrace();
			}
		}
		
		if(!loaded) {
			WaylandCraft.LOGGER.info("Native library could not be loaded from jar. Attempting to load from system");
			System.loadLibrary("waylandcraft");
		}
	}
	
	private WaylandCraftBridge(long handle) {
		this.instance = handle;
	}
	
	public static WaylandCraftBridge start() {
		long eglDisplay = GLFWNativeEGL.glfwGetEGLDisplay();
		if(eglDisplay == 0) {
			throw new RuntimeException("Failed to get EGL display!");
		}
		
		long handle = init(GLFW.Functions.GetProcAddress, eglDisplay);
		return new WaylandCraftBridge(handle);
	}
	
	protected WLCToplevel getOrCreateToplevel(long handle) {
		for(WLCToplevel toplevel : toplevels) {
			if(toplevel.getHandle() == handle) return toplevel;
		}
		WLCToplevel toplevel = new WLCToplevel(handle);
		// resolveSurface, called every frame including this one, sets the real
		// surface; the window starts surface-less so an X11 toplevel that has
		// no surface yet is not churned.
		toplevel.surface = NO_SURFACE;

		toplevels.add(toplevel);
		return toplevel;
	}
	
	public WLCToplevel[] getNewToplevels() {
		WLCToplevel[] toplevels = newToplevels.toArray(WLCToplevel[]::new);
		newToplevels.clear();
		
		return toplevels;
	}
	
	protected WLCPopup getOrCreatePopup(long handle) {
		for(WLCPopup popup : popups) {
			if(popup.getHandle() == handle) return popup;
		}
		WLCPopup popup = new WLCPopup(handle);
		// As with getOrCreateToplevel: resolveSurface sets the real surface.
		popup.surface = NO_SURFACE;

		popup.parentHandle = popupParent(this.instance, handle);

		popups.add(popup);
		return popup;
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
	
	private void deleteNonExistingPopups(long[] remainingHandles) {
		ArrayList<WLCPopup> popups_new = new ArrayList<WLCPopup>();
		for(WLCPopup popup : this.popups) {
			if(ArrayUtils.contains(remainingHandles, popup.getHandle())) {
				popups_new.add(popup);
			}
			else {
				freePopup(this.instance, popup.takeHandle());
			}
		}
		this.popups = popups_new;
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
	
	// Re-resolve a window's backing surface against the freshly queried handle.
	// An X11 window has no surface before the async xwayland-shell association
	// and loses it again on every unmap - smithay resets wl_surface() on each
	// X11 UnmapNotify - so the handle moves between the real surface and the
	// handle-0 surface over the window's life. This is honest per-frame
	// resolution: no holding, no frame counting. A window with no surface this
	// frame is surfaceless this frame; its framebuffer persists regardless (see
	// updateFramebuffers) and renders empty. For Wayland windows the handle is
	// stable frame to frame and this is a no-op.
	private void resolveSurface(WLCAbstractWindow window, long surfaceHandle) {
		window.surface = surfaceHandle == 0 ? NO_SURFACE : getOrCreateSurface(surfaceHandle);
	}

	private void findPopupParent(WLCPopup popup) {
		// Popups cannot change their parent, so if one is found, it's the one
		if(popup.parent != null) return;
		
		for(WLCToplevel toplevel : toplevels) {
			if(toplevel.getHandle() == popup.parentHandle) {
				popup.parent = toplevel;
				return;
			}
		}
		
		for(WLCPopup popup2 : popups) {
			if(popup2.getHandle() == popup.parentHandle) {
				popup.parent = popup2;
				return;
			}
		}
	}

	public void update() {
		ProfilerFiller profiler = Profiler.get();
		profiler.push("wayland");
		
		profiler.push("update");
		// Update wayland clients
		update(this.instance);
		profiler.pop();
		
		// Find all available toplevels and delete ones that no longer exist
		long[] toplevelHandles = toplevels(instance);
		deleteNonExistingToplevels(toplevelHandles);
		
		// Find all available popups and delete ones that no longer exist
		long[] popupHandles = popups(instance);
		deleteNonExistingPopups(popupHandles);
		
		long[] minimizeRequests = minimizeReq(instance);
		long[] maximizeRequests = maximizeReq(instance);
		long[] unmaximizeRequests = unmaximizeReq(instance);
		long[] fullscreenRequests = fullscreenReq(instance);
		long[] unfullscreenRequests = unfullscreenReq(instance);
		long[] fullscreened = fullscreened(instance);
		
		int[] moveRequest = moveRequest(instance);
		if(moveRequest != null) {
			lastMoveRequestSerial = moveRequest[0];
		}
		
		int[] resizeRequest = resizeRequest(instance);
		if(resizeRequest != null) {
			lastResizeRequest = new ResizeRequest(resizeRequest[0], resizeRequest[1]);
		}
		
		// Reset surface visited state
		for(WLCSurface surface : surfaces) {
			surface.visited = false;
		}
		
		profiler.push("surfaces");
		// Create new toplevels when necessary
		// Update surface tree geometry and properties of all toplevels
		for(long handle : toplevelHandles) {
			WLCToplevel toplevel = getOrCreateToplevel(handle);
			resolveSurface(toplevel, toplevelSurface(this.instance, handle));
			WLCSurface root = toplevel.getSurfaceTree();
			toplevel.lastChild = updateSurfaceTree(root);
			
			updateGeometry(toplevel);
			toplevel.title = toplevelTitle(toplevel.getHandle());
			toplevel.appID = toplevelAppID(toplevel.getHandle());
			
			if(ArrayUtils.contains(minimizeRequests, handle)) toplevel.requests.minimize = true;
			if(ArrayUtils.contains(maximizeRequests, handle)) toplevel.requests.maximize= true;
			if(ArrayUtils.contains(unmaximizeRequests, handle)) toplevel.requests.unmaximize = true;
			if(ArrayUtils.contains(fullscreenRequests, handle)) toplevel.requests.fullscreen = true;
			if(ArrayUtils.contains(unfullscreenRequests, handle)) toplevel.requests.unfullscreen = true;
			
			toplevel.fullscreen = ArrayUtils.contains(fullscreened, handle);
		}
		
		// Create new popups when necessary
		// Update surface tree geometry, parent relationships and offsets of all popups
		for(long handle : popupHandles) {
			WLCPopup popup = getOrCreatePopup(handle);
			resolveSurface(popup, popupSurface(this.instance, handle));
			findPopupParent(popup);
			
			int[] offset = popupOffset(instance, handle);
			popup.offsetX = offset[0];
			popup.offsetY = offset[1];
			
			WLCSurface root = popup.getSurfaceTree();
			popup.lastChild = updateSurfaceTree(root);
			updateGeometry(popup);
		}

		long dndIconHandle = dndIcon(instance);
		if(dndIconHandle != 0) {
			WLCSurface dndIconSurface = getOrCreateSurface(dndIconHandle);
			if(dndIcon != null && dndIcon.surface != dndIconSurface) dndIcon = null;
			if(dndIcon == null) dndIcon = new IconSurface(dndIconSurface);
			
			updateSurfaceData(instance, dndIcon.surface);
			dndIcon.surface.visited = true;
		}
		else {
			dndIcon = null;
		}
		
		// All surface trees have now been walked. Now delete all unvisited surfaces
		deleteUnvisitedSurfaces();
		profiler.pop();
		
		// Resolve surface parent handles to actual surfaces
		for(WLCSurface surface : surfaces) {
			if(surface.parentHandle != 0) {
				surface.parent = getOrCreateSurface(surface.parentHandle);
			}
			else {
				surface.parent = null;
			}
		}
		
		List<WLCAbstractWindow> allWindows = Stream.of(toplevels, popups).flatMap((l) -> l.stream()).collect(Collectors.toList());
		
		// Update all surface buffers
		for(WLCAbstractWindow window : allWindows) {
			WLCSurface root = window.getSurfaceTree();
			for(WLCSurface surface = root; surface != null; surface = surface.getNextChild()) {
				updateSurfaceData(instance, surface);
				calculateSubpos(surface);
			}
		}
		
		for(WLCToplevel toplevel : toplevels) {
			boolean mapped = toplevel.isMapped();
			if(mapped && !toplevel.wasMapped) {
				newToplevels.add(toplevel);
			}
			toplevel.wasMapped = mapped;
		}
		
		profiler.push("framebuffer");
		updateFramebuffers();
		profiler.pop();

		deleteNonExistingDmabufs(dmabufs(instance));
		
		updateFocusOrder();
		
		// Do client frame callbacks
		for(WLCSurface surface : surfaces) {
			sendFrame(surface.getHandle());
		}
		
		profiler.pop();
	}
	
	private void updateFramebuffers() {
		List<WLCAbstractWindow> allWindows = Stream.of(toplevels, popups).flatMap((l) -> l.stream()).collect(Collectors.toList());

		// One framebuffer per window for its whole life. It is created once,
		// when the window first has renderable content (a live surface tree),
		// and destroyed once, when the window is gone (see the cleanup below).
		// It is never destroyed/recreated on a surface change: each frame it is
		// re-pointed at the window's current surface tree and re-rendered into
		// the SAME target under the SAME texture Identifier. A window that is
		// currently surfaceless (an X11 window between unmap and remap) keeps
		// its framebuffer and renders empty/transparent - no texture churn.
		for(WLCAbstractWindow window : allWindows) {
			if(window.framebuffer == null) {
				if(window.getSurfaceTree() == null || !window.getSurfaceTree().isAlive()) continue;
				window.framebuffer = new WindowFramebuffer(window.getSurfaceTree());
				framebuffers.add(window.framebuffer);
			}
			window.framebuffer.setSurfaceTree(window.getSurfaceTree());
			window.framebuffer.render();
		}

		// Render dnd icon
		if(dndIcon != null) {
			if(dndIcon.framebuffer == null) {
				dndIcon.framebuffer = new WindowFramebuffer(dndIcon.surface);
				framebuffers.add(dndIcon.framebuffer);
			}
			dndIcon.framebuffer.setSurfaceTree(dndIcon.surface);
			dndIcon.framebuffer.render();
		}

		// Cleanup framebuffers no longer owned by a live window or the dnd icon.
		// Keyed on the owning window, not surfaceTree liveness: a tracked X11
		// window keeps its framebuffer across an unmap and loses it only when
		// the window itself is gone.
		Set<WindowFramebuffer> usedFramebuffers = new HashSet<WindowFramebuffer>();
		for(WLCAbstractWindow window : allWindows) {
			if(window.framebuffer != null) usedFramebuffers.add(window.framebuffer);
		}
		if(dndIcon != null && dndIcon.framebuffer != null) usedFramebuffers.add(dndIcon.framebuffer);
		for(WindowFramebuffer framebuffer : framebuffers) {
			if(!usedFramebuffers.contains(framebuffer)) framebuffer.destroy();
		}
		framebuffers.retainAll(usedFramebuffers);

		WindowFramebuffer.endFrame();
	}
	
	private void updateGeometry(WLCAbstractWindow window) {
		int[] data = surfaceXDGGeometry(window.surface.getHandle());
		SurfaceGeometry geometry;
		
		if(data == null) {
			geometry = new SurfaceGeometry(0, 0, window.surface.width(), window.surface.height());
		}
		else {
			geometry = new SurfaceGeometry(data[0], data[1], data[2], data[3]);
		}
		
		window.geometry = geometry;
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
	
	public WLCToplevel[] getMappedToplevels() {
		return toplevels.stream().filter((t) -> t.isMapped()).toArray(WLCToplevel[]::new);
	}
	
	public WLCToplevel getToplevel(long handle) {
		return toplevels.stream().filter((w) -> w.getHandle() == handle).findAny().orElse(null);
	}
	
	public WLCPopup[] getPopups() {
		return popups.toArray(new WLCPopup[popups.size()]);
	}
	
	public WLCPopup[] getMappedPopups() {
		return popups.stream().filter((t) -> t.isMapped()).toArray(WLCPopup[]::new);
	}
	
	public String getSocket() {
		return socket(this.instance);
	}
	
	public boolean inputRegionContains(WLCSurface surface, double x, double y) {
		return checkInputRegion(surface.getHandle(), x, y);
	}
	
	public void sendMotion(double x, double y) {
		pointerMotion(instance, x, y);
	}
	
	public void sendMotionRefocus(WLCSurface surface, double x, double y) {
		pointerMotionFocus(instance, surface.getHandle(), x, y);
	}
	
	public void sendRelativeMotion(double dx, double dy) {
		pointerRelMotion(instance, dx, dy);
	}
	
	public void sendMotionOutside() {
		pointerLeave(instance);
	}
	
	public boolean maybeLockPointer(WLCSurface surface) {
		return maybePointerLock(instance, surface.getHandle());
	}
	
	public void unlockPointer() {
		pointerUnlock(instance);
	}
	
	public int sendButton(int button, int state) {
		return pointerButton(instance, button, state);
	}
	
	public void sendScroll(int axis, double value) {
		pointerAxis(instance, axis, value);
	}
	
	public CursorShape getCursorShape() {
		return CursorShape.fromId(cursorShape(instance));
	}
	
	public void focusSurface(@Nullable WLCToplevel toplevel) {
		long handle = 0;
		if(toplevel != null) {
			handle = toplevel.getHandle();
		}
		
		keyboardFocus(instance, handle);
		
		// Make toplevel most recently focused
		if(toplevel != null) {
			focusOrder.remove(toplevel);
			focusOrder.addLast(toplevel);
		}
	}
	
	public void activateKeyboard() {
		keyboardActivate(instance);
	}
	
	public void deactivateKeyboard() {
		keyboardDeactivate(instance);
	}
	
	private void updateFocusOrder() {
		focusOrder.removeIf((t) -> !toplevels.contains(t));
		for(WLCToplevel toplevel : toplevels) {
			if(!focusOrder.contains(toplevel)) focusOrder.addLast(toplevel);
		}
	}
	
	// Find the most recently focused toplevel that exists
	public WLCToplevel getMostRecentFocus() {
		updateFocusOrder();
		return focusOrder.peekLast();
	}
	
	// Find the most recently focused toplevel that exists
	public Stream<WLCToplevel> getMostToLeastRecentFocus() {
		updateFocusOrder();
		return focusOrder.reversed().stream();
	}
	
	public void pressKey(int scancode) {
		keyboardInput(instance, scancode, 1);
	}
	
	public void releaseKey(int scancode) {
		keyboardInput(instance, scancode, 0);
	}
	
	public void internalKeyUpdate(int scancode, boolean pressed) {
		keyboardUpdate(instance, scancode, pressed);
	}
	
	public void resizeToplevelInteractive(WLCToplevel toplevel, int width, int height) {
		toplevelResize(toplevel.getHandle(), width, height, true);
	}
	
	public void resizeToplevel(WLCToplevel toplevel, int width, int height) {
		toplevelResize(toplevel.getHandle(), width, height, false);
	}
	
	public void resizeToplevelOverride(WLCToplevel toplevel, int width, int height) {
		toplevelResizeOvr(toplevel.getHandle(), width, height);
	}

	public void closeToplevel(WLCToplevel toplevel) {
		toplevelClose(toplevel.getHandle());
	}
	
	public void maximizeToplevel(WLCToplevel toplevel) {
		toplevelMaximize(instance, toplevel.getHandle());
	}
	
	public void fullscreenToplevel(WLCToplevel toplevel) {
		toplevelFullscreen(instance, toplevel.getHandle(), toplevel.geometry.width(), toplevel.geometry.height());
	}
	
	public Integer checkMoveRequest() {
		if(lastMoveRequestSerial == null) return null;
		int serial = lastMoveRequestSerial.intValue();
		lastMoveRequestSerial = null;
		return serial;
	}
	
	public ResizeRequest checkResizeRequest() {
		if(lastResizeRequest == null) return null;
		ResizeRequest req = lastResizeRequest;
		lastResizeRequest = null;
		return req;
	}
	
	public void resizeOutput(int width, int height) {
		outputResize(instance, width, height);
	}
	
	public void setOutputBounds(int width, int height) {
		outputSetBounds(instance, width, height);
	}
	
	public Size getOutputSize() {
		int[] size = outputSize(instance);
		return new Size(size[0], size[1]);
	}
	
	public Size getOutputBounds() {
		int[] size = outputBounds(instance);
		return new Size(size[0], size[1]);
	}
	
	public RawDesktopEntry loadDesktopEntry(File path) {
		return loadDesktopEntry(instance, path.getAbsolutePath());
	}
	
	public RawDesktopEntry[] loadSystemDesktopEntries() {
		return loadDesktopEntries(instance);
	}

	public @Nullable String resolveAppID(String appId) {
		if(appId == null) return null;
		return resolveAppID(instance, appId);
	}
	
	public boolean renderSVG(File file, int width, int height, long ptr) {
		return renderSVG(file.getAbsolutePath(), width, height, ptr);
	}
	
	public boolean execApp(String appId) {
		return execApp(instance, appId);
	}
	
	public void setKeymapDefault() {
		setKeymapDefault(instance);
	}
	
	public String exportKeymap() {
		return exportKeymap(instance);
	}
	
	public boolean setKeymapFromStr(String keymap) {
		return setKeymapFromStr(instance, keymap);
	}
	
	public Integer checkDndRequest() {
		int[] serial = checkDndRequest(instance);
		if(serial == null) return null;
		return serial[0];
	}

	// True while an X11 app is dragging onto WaylandCraft (Stage C). Polled to
	// start/stop an in-world X11DNDGrab that drives the Wayland target.
	public boolean isX11DndActive() {
		return checkX11Dnd(instance);
	}

	public void dndCancel() {
		dndCancel(instance);
	}
	
	public void dndDrop() {
		dndDrop(instance);
	}
	
	public void sendDndMotion(WLCSurface surface, double x, double y) {
		long handle = surface == null ? 0 : surface.getHandle();
		dndMotion(instance, handle, x, y);
	}
	
	public static record Size(int width, int height) {}
	
	public static record ResizeRequest(int serial, int edges) {}
	
	private static native long init(long glfwGetProcAddress, long eglDisplay);
	private static native void update(long instance);
	private static native String socket(long instance);
	private static native void sendFrame(long handle);
	
	private static native void updateSurfaceData(long instance, WLCSurface surface);
	
	private static native long[] toplevels(long instance);
	private static native long toplevelSurface(long instance, long handle);
	private static native String toplevelTitle(long handle);
	private static native String toplevelAppID(long handle);
	// Resize toplevel
	private static native void toplevelResize(long handle, int width, int height, boolean interactive);
	// Resize toplevel override, keep maximized and fullscreen state, stop interactive resize
	private static native void toplevelResizeOvr(long handle, int width, int height);
	// Request a toplevel to close (sends xdg_toplevel.close to the client)
	private static native void toplevelClose(long handle);
	
	// Collect all toplevels that have sent a minimize request and clear the list
	private static native long[] minimizeReq(long instance);
	// Collect all toplevels that have sent a maximize request and clear the list
	private static native long[] maximizeReq(long instance);
	// Collect all toplevels that have sent an unmaximize request and clear the list
	private static native long[] unmaximizeReq(long instance);
	// Collect all toplevels that have sent a fullscreen request and clear the list
	private static native long[] fullscreenReq(long instance);
	// Collect all toplevels that have sent an unfullscreen request and clear the list
	private static native long[] unfullscreenReq(long instance);
	
	// Collect up to one serial of a sent interactive move request
	private static native int[] moveRequest(long instance);
	// Collect up to one serial of a sent interactive resize request
	private static native int[] resizeRequest(long instance);
	
	// All toplevels that are currently in fullscreen
	private static native long[] fullscreened(long instance);
	
	private static native void toplevelMaximize(long instance, long handle);
	private static native void toplevelFullscreen(long instance, long handle, int width, int height);
	
	private static native long[] popups(long instance);
	private static native long popupSurface(long instance, long handle);
	// Query the parent of a popup
	// Returned handle is a handle either to a toplevel or another popup
	private static native long popupParent(long instance, long handle);
	// Query popup local offset coordinates
	// Returns two-element list containing x,y
	private static native int[] popupOffset(long instance, long handle);
	
	// Query the xdg_surface window geometry of a toplevel or popup.
	// handle should be the handle to the root WLCSurface
	// Returns four-element array containing x,y,width,height which could be null
	private static native int[] surfaceXDGGeometry(long handle);
	
	private static native long[] dmabufs(long instance);
	
	// Updates the surface tree given by the root surface
	// This changes the doubly linked list of the WLCSurfaces.
	// The returned surface is the last (most deeply nested) child
	private native WLCSurface updateSurfaceTree(WLCSurface root);
	
	// Check if point in surface input region
	private static native boolean checkInputRegion(long surfaceHandle, double x, double y);
	
	// Create pointer motion event
	private static native void pointerMotion(long instance, double x, double y);
	
	// Create pointer motion event
	private static native void pointerMotionFocus(long instance, long handle, double x, double y);
	
	// Send relative pointer motion to surface with pointer focus
	private static native void pointerRelMotion(long instance, double dx, double dy);
	
	private static native boolean maybePointerLock(long instance, long handle);
	
	private static native void pointerUnlock(long instance);
	
	// Remove pointer focus from all surfaces
	private static native void pointerLeave(long instance);
	
	// Create pointer button event. `button` has to be the linux button code, state is 1 for pressed, 0 for released
	private static native int pointerButton(long instance, int button, int state);
	
	// Create pointer axis event. `axis` is the scroll axis (0 for vertical, 1 for horizontal)
	private static native void pointerAxis(long instance, int axis, double value);
	
	// Get active cursor image
	private static native int cursorShape(long instance);
	
	// Set keyboard focus to a wayland surface. The handle may be 0 to unfocus any surfaces
	private static native void keyboardFocus(long instance, long surfaceHandle);
	
	private static native void keyboardActivate(long instance);
	private static native void keyboardDeactivate(long instance);
	
	// Keyboard input. scancode is the raw keycode. action: 0 is released, 1 is pressed.
	private static native void keyboardInput(long instance, int scancode, int action);
	
	// Update internal key state
	private static native void keyboardUpdate(long instance, int scancode, boolean pressed);
	
	private static native int[] outputSize(long instance);
	private static native int[] outputBounds(long instance);
	
	// Update virtual output dimensions
	private static native void outputResize(long instance, int width, int height);
	
	// Update virtual output maximum window bounds
	private static native void outputSetBounds(long instance, int width, int height);
	
	private static native void freeSurface(long instance, long handle);
	private static native void freeToplevel(long instance, long handle);
	private static native void freePopup(long instance, long handle);
	
	private static native RawDesktopEntry loadDesktopEntry(long instance, String path);
	private static native RawDesktopEntry[] loadDesktopEntries(long instance);
	// Resolve a Wayland app-id or X11 WM_CLASS to a desktop-entry id, or null
	private static native String resolveAppID(long instance, String appId);
	
	private static native boolean renderSVG(String path, int width, int height, long ptr);
	
	private static native boolean execApp(long instance, String appId);
	
	private static native void setKeymapDefault(long instance);
	private static native String exportKeymap(long instance);
	private static native boolean setKeymapFromStr(long instance, String keymap);
	
	private static native int[] checkDndRequest(long instance);
	private static native boolean checkDndActive(long instance);
	// True while an X11-initiated drag is in flight onto WaylandCraft
	private static native boolean checkX11Dnd(long instance);
	private static native void dndCancel(long instance);
	private static native void dndDrop(long instance);
	private static native void dndMotion(long instance, long surface, double x, double y);
	private static native long dndIcon(long instance);
	
}
