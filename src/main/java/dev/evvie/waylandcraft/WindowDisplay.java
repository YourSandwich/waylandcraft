package dev.evvie.waylandcraft;

import org.joml.Matrix3d;
import org.joml.Vector3d;

import dev.evvie.waylandcraft.bridge.WLCAbstractWindow;
import dev.evvie.waylandcraft.bridge.WLCPopup;
import dev.evvie.waylandcraft.bridge.WLCSurface;
import dev.evvie.waylandcraft.bridge.WLCToplevel;
import net.fabricmc.fabric.api.client.rendering.v1.WorldRenderContext;
import net.minecraft.world.phys.Vec2;
import net.minecraft.world.phys.Vec3;

public class WindowDisplay {
	
	private static final float PIXEL_SCALE = 1.0f / 500;
	
	public final WLCAbstractWindow window;
	
	// World position of window
	public Vec3 pivot = new Vec3(0, 0, 0);
	
	// Window facing direction normal
	private Vec3 normal = new Vec3(0, 0, 1);
	
	// Window orientation downwards vector, has to be orthogonal to `normal` and normalized
	private Vec3 down = new Vec3(0, -1, 0);
	
	private int width;
	private int height;
	
	public WindowDisplay(WLCToplevel toplevel) {
		this.window = toplevel;
	}
	
	public WindowDisplay(WLCPopup popup) {
		this.window = popup;
	}
	
	public boolean isAlive() {
		return window.isAlive();
	}
	
	public void rotate(Vec3 normal, Vec3 down) {
		this.normal = normal;
		this.down = down;
	}
	
	public Vec3 normal() {
		return normal;
	}
	
	public Vec3 down() {
		return down;
	}
	
	public Vec3 right() {
		return normal.cross(down);
	}
	
	public Vec3 localX() {
		return right().scale(PIXEL_SCALE);
	}
	
	public Vec3 localY() {
		return down.scale(PIXEL_SCALE);
	}
	
	// World coordinates of the origin of the root surface surface-local coordinate space
	public Vec3 origin() {
		return pivot.add(localX().scale(-width/2)).add(localY().scale(-height/2));
	}
	
	public Vec3 localToWorld(double x, double y, double z) {
		Vec3 origin = origin();
		Vec3 localX = localX();
		Vec3 localY = localY();
		return origin.add(localX.scale(x)).add(localY.scale(y)).add(normal.scale(z));
	}
	
	public void moveOrigin(Vec3 pos) {
		pivot = pos.add(localX().scale(width/2)).add(localY().scale(height/2));
	}
	
	private void updateGeometry() {
		WLCSurface root = window.getSurfaceTree();
		width = root.width();
		height = root.height();
	}
	
	public void render(WorldRenderContext ctx) {
		updateGeometry();
		
		int xoff = window.framebuffer.getXOff();
		int yoff = window.framebuffer.getYOff();
		int bufWidth = window.framebuffer.getWidth();
		int bufHeight = window.framebuffer.getHeight();
		
		Vec3 origin = origin();
		Vec3 localX = localX();
		Vec3 localY = localY();
		
		Vec3 bufOrigin = origin.add(localX.scale(-xoff)).add(localY.scale(-yoff));
		
		Vec3 tl = bufOrigin;
		Vec3 bl = bufOrigin.add(localY.scale(bufHeight));
		Vec3 br = bl.add(localX.scale(bufWidth));
		Vec3 tr = tl.add(localX.scale(bufWidth));
		
		RenderUtils.drawTexturedQuad(ctx.camera(), window.framebuffer.getTexture(), tl, bl, br, tr, new Vec2(0, 0), new Vec2(0, 1), new Vec2(1, 1), new Vec2(1, 0));
		RenderUtils.drawCutoutColorlessQuad(ctx.camera(), window.framebuffer.getTexture(), tr, br, bl, tl, new Vec2(1, 0), new Vec2(1, 1), new Vec2(0, 1), new Vec2(0, 0));
	}
	
	public WindowBounds calculateBounds() {
		WindowBounds bounds = new WindowBounds();
		WLCSurface surface;
		
		for(surface = window.getSurfaceTree(); surface != null; surface = surface.getNextChild()) {
			int minX = surface.xSubpos;
			int minY = surface.ySubpos;
			int maxX = minX + surface.width();
			int maxY = minY + surface.height();
			
			if(minX < bounds.minX) bounds.minX = minX;
			if(minY < bounds.minY) bounds.minY = minY;
			if(maxX > bounds.maxX) bounds.maxX = maxX;
			if(maxY > bounds.maxY) bounds.maxY = maxY;
		}
		
		return bounds;
	}
	
	/* Transform absolute world coordinates to surface-local pixel coordinates relative to toplevel (0, 0)
	 * 
	 * The resulting vector is the (x, y) pixel location and the z value is the block distance normal to the plane.
	 */
	public Vec3 worldToLocal(Vec3 in) {
		Vec3 origin = origin();
		Vec3 localX = localX();
		Vec3 localY = localY();
		
		// World coordinates relative to the origin of this window
		Vec3 world = in.subtract(origin);
		
		Matrix3d matrix = new Matrix3d(
			localX.x, localX.y, localX.z, // Column 0
			localY.x, localY.y, localY.z, // Column 1
			normal.x, normal.y, normal.z  // Column 2
		);
		matrix.invert();
		
		Vector3d result = matrix.transform(new Vector3d(world.x, world.y, world.z));
		return new Vec3(result.x, result.y, result.z);
	}
	
	/* Perform ray-window intersection
	 * `dir` must be normalized.
	 */
	public DisplayHitResult intersect(Vec3 pos, Vec3 dir) {
		double p1 = pivot.subtract(pos).dot(normal);
		double p2 = dir.dot(normal);
		
		// Avoid division by zero
		if(p2 == 0) return null;
		
		double t = p1 / p2;
		
		// Intersection happens behind the camera
		if(t < 0) return null;
		
		Vec3 hitPos = pos.add(dir.scale(t));
		Vec3 localCoords = worldToLocal(hitPos);
		
		WindowBounds bounds = calculateBounds();
		
		// Completely outside of window extent
		if(!bounds.contains((int) localCoords.x, (int) localCoords.y)) return null;
		
		// Flip z-coordinate when on the window backside
		double dist = t;
		if(p2 > 0) dist *= -1;
		
		return new DisplayHitResult(this, hitPos, localCoords, dist);
	}
	
	public static class WindowBounds {
		
		public int minX;
		public int minY;
		public int maxX;
		public int maxY;
		
		public WindowBounds(int minX, int minY, int maxX, int maxY) {
			this.minX = minX;
			this.minY = minY;
			this.maxX = maxX;
			this.maxY = maxY;
		}
		
		public WindowBounds() {
			this(0, 0, 0, 0);
		}
		
		public boolean contains(int x, int y) {
			return x >= minX && x <= maxX && y >= minY && y <= maxY;
		}
		
	}
	
	public static class DisplayHitResult {
		
		public final WindowDisplay target;
		public final Vec3 position;
		public final Vec3 surfaceLocal;
		public final double dist;
		
		public DisplayHitResult(WindowDisplay target, Vec3 position, Vec3 surfaceLocal, double dist) {
			this.target = target;
			this.position = position;
			this.surfaceLocal = surfaceLocal;
			this.dist = dist;
		}
		
	}
	
}
