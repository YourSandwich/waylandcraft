package dev.evvie.waylandcraft.render;

import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.OptionalInt;

import org.joml.Matrix4fc;

import com.mojang.blaze3d.buffers.GpuBuffer;
import com.mojang.blaze3d.buffers.GpuBufferSlice;
import com.mojang.blaze3d.buffers.Std140Builder;
import com.mojang.blaze3d.buffers.Std140SizeCalculator;
import com.mojang.blaze3d.pipeline.BlendFunction;
import com.mojang.blaze3d.pipeline.ColorTargetState;
import com.mojang.blaze3d.pipeline.RenderPipeline;
import com.mojang.blaze3d.pipeline.RenderTarget;
import com.mojang.blaze3d.pipeline.TextureTarget;
import com.mojang.blaze3d.platform.DestFactor;
import com.mojang.blaze3d.platform.SourceFactor;
import com.mojang.blaze3d.shaders.UniformType;
import com.mojang.blaze3d.systems.RenderPass;
import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.blaze3d.textures.FilterMode;
import com.mojang.blaze3d.textures.GpuTextureView;
import com.mojang.blaze3d.vertex.BufferBuilder;
import com.mojang.blaze3d.vertex.ByteBufferBuilder;
import com.mojang.blaze3d.vertex.DefaultVertexFormat;
import com.mojang.blaze3d.vertex.MeshData;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexFormat;

import dev.evvie.waylandcraft.WaylandCraft;
import dev.evvie.waylandcraft.bridge.WLCSurface;
import dev.evvie.waylandcraft.bridge.WLCSurface.ViewportSource;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.DynamicUniformStorage;
import net.minecraft.client.renderer.DynamicUniformStorage.DynamicUniform;
import net.minecraft.client.renderer.RenderPipelines;
import net.minecraft.client.renderer.texture.AbstractTexture;
import net.minecraft.resources.Identifier;

/*
 * One framebuffer per window, alive for the window's whole life. Created once
 * when the window first has renderable content (updateFramebuffers) and
 * destroyed once when the window leaves the model. It is NEVER destroyed and
 * recreated on a surface change.
 *
 * The texture Identifier and its FramebufferTexture are registered in the
 * TextureManager once and never unregistered until destroy(); the in-world
 * quad therefore samples a stable key for the window's whole life - no
 * missing-texture swap, no content-swap flicker. On a surface change the
 * framebuffer is just re-pointed (setSurfaceTree) and re-rendered into the
 * SAME target. A surfaceless window renders empty/transparent and keeps its
 * target. A genuine size change allocates a new RenderTarget and re-points the
 * existing FramebufferTexture at it under the SAME Identifier, registering the
 * new backing before the old is dropped, so no frame has a dangling key.
 */
public class WindowFramebuffer {

	public static final RenderPipeline WINDOW_PIPELINE = RenderPipelines.register(
		RenderPipeline.builder()
		.withLocation(Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, "pipeline/window"))
		.withVertexShader(Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, "window"))
		.withFragmentShader(Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, "window"))
		.withVertexFormat(DefaultVertexFormat.POSITION_TEX, VertexFormat.Mode.QUADS)
		.withSampler("sampler")
		.withUniform("window_info", UniformType.UNIFORM_BUFFER)
		.withColorTargetState(new ColorTargetState(new BlendFunction(SourceFactor.ONE, DestFactor.ONE_MINUS_SRC_ALPHA)))
		.withCull(false)
		.build()
	);

	private static DynamicUniformStorage<WindowInfoUniform> uniformStorage = null;

	// Per-framebuffer unique id. Object.hashCode() is not collision-free, and a
	// collision makes two framebuffers register the same texture Identifier -
	// they then overwrite each other in the TextureManager.
	private static int nextId = 0;

	private final int id = nextId++;

	// The window's current surface tree. Re-pointed on a surface change (X11
	// oscillation, or a normal subsurface change); never makes the framebuffer
	// itself get torn down. Null when the window currently has no surface.
	private WLCSurface surfaceTree;

	// Stable for the framebuffer's whole life: registered in the TextureManager
	// once in the constructor, released once in destroy(). The backing
	// RenderTarget is swapped under this same key on a size change.
	private final Identifier location;
	private final FramebufferTexture texture;
	private RenderTarget target;

	private int width;
	private int height;
	private int xoff;
	private int yoff;

	public WindowFramebuffer(WLCSurface surfaceTree) {
		this.surfaceTree = surfaceTree;
		measure();

		// The target needs a non-zero size; a window can be created surfaceless
		// (a just-mapped X11 window between its UnmapNotify and the next commit).
		this.target = new TextureTarget(name(), Math.max(width, 1), Math.max(height, 1), false);
		this.texture = new FramebufferTexture(target.getColorTextureView());
		this.location = Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, name());
		Minecraft.getInstance().getTextureManager().register(location, texture);
	}

	public static void endFrame() {
		if(uniformStorage != null) uniformStorage.endFrame();
	}

	private static void ensureUniformStorage() {
		if(uniformStorage == null) {
			uniformStorage = new DynamicUniformStorage<WindowInfoUniform>("window framebuffer", WindowInfoUniform.SIZE, 2);
		}
	}

	// Re-point this framebuffer at the window's current surface tree. Cheap and
	// safe to call every frame; null means the window is currently surfaceless.
	public void setSurfaceTree(WLCSurface surfaceTree) {
		this.surfaceTree = surfaceTree;
	}

	// Recompute width/height/xoff/yoff from the current surface tree. An empty
	// or null tree collapses to a zero-size, zero-offset window.
	private void measure() {
		int minX = 0;
		int minY = 0;
		int maxX = 0;
		int maxY = 0;

		for(WLCSurface surface = surfaceTree; surface != null; surface = surface.getNextChild()) {
			int sMinX = surface.xSubpos;
			int sMinY = surface.ySubpos;
			int sMaxX = sMinX + surface.width();
			int sMaxY = sMinY + surface.height();

			if(sMinX < minX) minX = sMinX;
			if(sMinY < minY) minY = sMinY;
			if(sMaxX > maxX) maxX = sMaxX;
			if(sMaxY > maxY) maxY = sMaxY;
		}

		this.xoff = -minX;
		this.yoff = -minY;
		this.width = maxX - minX;
		this.height = maxY - minY;
	}

	private String name() {
		return "wayland-framebuffer-" + id;
	}

	public void render() {
		measure();

		int targetWidth = Math.max(width, 1);
		int targetHeight = Math.max(height, 1);

		// Genuine size change: allocate a new backing target and re-point the
		// existing FramebufferTexture at it under the SAME Identifier. The new
		// backing is registered before the old target is dropped, so no frame
		// ever samples a missing/dangling texture for this window.
		if(targetWidth != target.width || targetHeight != target.height) {
			RenderTarget old = target;
			target = new TextureTarget(name(), targetWidth, targetHeight, false);
			texture.repoint(target.getColorTextureView());
			Minecraft.getInstance().getTextureManager().register(location, texture);
			old.destroyBuffers();
		}

		PoseStack poseStack = new PoseStack();
		poseStack.translate(-1.0, -1.0, 0.0);
		poseStack.scale(2.0f / targetWidth, 2.0f / targetHeight, 1.0f);

		ArrayList<CompiledBufferDraw> elements = new ArrayList<>();
		for(WLCSurface surface = surfaceTree; surface != null; surface = surface.getNextChild()) {
			BufferDraw draw = bakeSurface(surface, xoff + surface.xSubpos, yoff + surface.ySubpos);
			if(draw != null) elements.add(draw.compile());
		}

		ensureUniformStorage();
		GpuBufferSlice alphaUniforms = uniformStorage.writeUniform(new WindowInfoUniform(poseStack.last().pose(), true));
		GpuBufferSlice opaqueUniforms = uniformStorage.writeUniform(new WindowInfoUniform(poseStack.last().pose(), false));

		try {
			// The render pass clears to transparent first; with no elements
			// (surfaceless window) that leaves a clean transparent target.
			try(RenderPass pass = RenderSystem.getDevice().createCommandEncoder().createRenderPass(() -> "window framebuffer", target.getColorTextureView(), OptionalInt.of(0x00000000))) {
				pass.setPipeline(WINDOW_PIPELINE);
				for(CompiledBufferDraw element : elements) {
					pass.setUniform("window_info", element.alpha ? alphaUniforms : opaqueUniforms);
					pass.bindTexture("sampler", element.textureView, RenderSystem.getSamplerCache().getClampToEdge(FilterMode.NEAREST));
					pass.setVertexBuffer(0, element.vertexBuffer);
					pass.setIndexBuffer(element.indexBuffer, element.indexType);
					pass.drawIndexed(0, 0, element.indexCount, 1);
				}
			}
		}
		finally {
			for(CompiledBufferDraw element : elements) {
				element.vertexBuffer.close();
			}
		}
	}

	private BufferDraw bakeSurface(WLCSurface surface, float x, float y) {
		BufferTexture buf = surface.getBuffer();
		if(buf == null) return null;

		float w = surface.width();
		float h = surface.height();

		float crop_x1 = 0.0f;
		float crop_y1 = 0.0f;
		float crop_x2 = 1.0f;
		float crop_y2 = 1.0f;

		ViewportSource src = surface.getViewportSource();
		if(src != null) {
			crop_x1 = (float) (src.x / buf.width);
			crop_y1 = (float) (src.y / buf.height);
			crop_x2 = (float) ((src.x + src.width) / buf.width);
			crop_y2 = (float) ((src.y + src.height) / buf.height);
		}

		return new BufferDraw(buf.textureView, x, y, w, h, crop_x1, crop_y1, crop_x2, crop_y2, buf.format != BufferTexture.FORMAT_XRGB8888);
	}

	private static record CompiledBufferDraw(GpuTextureView textureView, GpuBuffer vertexBuffer, GpuBuffer indexBuffer, int indexCount, VertexFormat.IndexType indexType, boolean alpha) {
	}

	private static record BufferDraw(GpuTextureView textureView, float x, float y, float w, float h, float u1, float v1, float u2, float v2, boolean alpha) {

		public CompiledBufferDraw compile() {
			try(ByteBufferBuilder byteBuilder = new ByteBufferBuilder(DefaultVertexFormat.POSITION_TEX.getVertexSize() * 4)) {
				BufferBuilder builder = new BufferBuilder(byteBuilder, VertexFormat.Mode.QUADS, DefaultVertexFormat.POSITION_TEX);
				builder.addVertex(x, y, 0).setUv(u1, v1);
				builder.addVertex(x + w, y, 0).setUv(u2, v1);
				builder.addVertex(x + w, y + h, 0).setUv(u2, v2);
				builder.addVertex(x, y + h, 0).setUv(u1, v2);

				try(MeshData mesh = builder.buildOrThrow()) {
					int indexCount = mesh.drawState().indexCount();
					RenderSystem.AutoStorageIndexBuffer indices = RenderSystem.getSequentialBuffer(VertexFormat.Mode.QUADS);
					GpuBuffer vertexBuffer = RenderSystem.getDevice().createBuffer(null, GpuBuffer.USAGE_VERTEX | GpuBuffer.USAGE_COPY_DST, mesh.vertexBuffer());
					GpuBuffer indexBuffer = indices.getBuffer(indexCount);
					return new CompiledBufferDraw(textureView, vertexBuffer, indexBuffer, indexCount, indices.type(), alpha);
				}
			}
		}

	}

	// Free the GPU target and release the Identifier. Called once, when the
	// owning window is gone.
	public void destroy() {
		target.destroyBuffers();
		Minecraft.getInstance().getTextureManager().release(location);
	}

	public int getWidth() {
		return width;
	}

	public int getHeight() {
		return height;
	}

	public int getXOff() {
		return xoff;
	}

	public int getYOff() {
		return yoff;
	}

	public GpuTextureView getTextureView() {
		return target.getColorTextureView();
	}

	public Identifier getTextureLocation() {
		return location;
	}

	private static class FramebufferTexture extends AbstractTexture {

		public FramebufferTexture(GpuTextureView textureView) {
			repoint(textureView);
		}

		// Swap the backing texture on a size change. The TextureManager keeps
		// the same FramebufferTexture object, so registering it again is a
		// no-op put and the old GpuTextureView/GpuTexture are owned by the old
		// RenderTarget, which destroys them itself.
		public void repoint(GpuTextureView textureView) {
			this.textureView = textureView;
			this.texture = textureView.texture();
		}

		@Override
		public void close() {
		}

	}

	private static record WindowInfoUniform(Matrix4fc mat, boolean alpha) implements DynamicUniform {

		public static final int SIZE = new Std140SizeCalculator().putMat4f().putFloat().get();

		@Override
		public void write(ByteBuffer byteBuffer) {
			Std140Builder.intoBuffer(byteBuffer).putMat4f(mat).putFloat(alpha ? 0.0f : 1.0f);
		}

	}

}
