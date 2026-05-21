package dev.evvie.waylandcraft.render;

import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.List;
import java.util.OptionalDouble;
import java.util.OptionalInt;

import org.joml.Matrix4f;
import org.joml.Matrix4fc;

import com.mojang.blaze3d.buffers.GpuBuffer;
import com.mojang.blaze3d.buffers.GpuBufferSlice;
import com.mojang.blaze3d.buffers.Std140Builder;
import com.mojang.blaze3d.buffers.Std140SizeCalculator;
import com.mojang.blaze3d.pipeline.DepthStencilState;
import com.mojang.blaze3d.pipeline.RenderPipeline;
import com.mojang.blaze3d.pipeline.RenderTarget;
import com.mojang.blaze3d.platform.CompareOp;
import com.mojang.blaze3d.shaders.UniformType;
import com.mojang.blaze3d.systems.RenderPass;
import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.blaze3d.textures.GpuTexture;
import com.mojang.blaze3d.textures.GpuTextureView;
import com.mojang.blaze3d.textures.TextureFormat;
import com.mojang.blaze3d.vertex.BufferBuilder;
import com.mojang.blaze3d.vertex.ByteBufferBuilder;
import com.mojang.blaze3d.vertex.DefaultVertexFormat;
import com.mojang.blaze3d.vertex.MeshData;
import com.mojang.blaze3d.vertex.VertexFormat;

import dev.evvie.waylandcraft.WaylandCraft;
import net.fabricmc.fabric.api.client.rendering.v1.level.LevelRenderEvents;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.DynamicUniformStorage;
import net.minecraft.client.renderer.DynamicUniformStorage.DynamicUniform;
import net.minecraft.resources.Identifier;
import net.minecraft.world.phys.Vec3;

/*
 * Post-composite render path for in-world windows under an Iris shaderpack.
 * Iris ignores mod custom shaders during the world pass, so windows are drawn
 * straight onto the main framebuffer after Iris finishes compositing -
 * GameRendererMixin calls drawPending() at the return of renderLevel - and so
 * keep their exact texture colors, untouched by the shaderpack.
 *
 * World-space windows (placed displays, item frames) depth-test against the
 * scene; view-space windows (held in hand) skip the depth test, drawn as an
 * always-visible overlay. The scene depth is copied out at END_MAIN because the
 * main render target's depth is empty by drawPending().
 */
public class ShaderWindowPass {

	private static final RenderPipeline.Snippet WINDOW_SNIPPET = RenderPipeline.builder()
			.withVertexShader(Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, "core/window_world"))
			.withFragmentShader(Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, "core/window_world"))
			.withSampler("Sampler0")
			.withUniform("window_mvp", UniformType.UNIFORM_BUFFER)
			.withVertexFormat(DefaultVertexFormat.POSITION_TEX, VertexFormat.Mode.QUADS)
			.buildSnippet();

	// World windows depth-test against the scene; in-hand windows always draw on top.
	private static final DepthStencilState WORLD_DEPTH = new DepthStencilState(CompareOp.LESS_THAN_OR_EQUAL, true);
	private static final DepthStencilState OVERLAY_DEPTH = new DepthStencilState(CompareOp.ALWAYS_PASS, false);

	private static final RenderPipeline WORLD_FRONT = RenderPipeline.builder(WINDOW_SNIPPET)
			.withLocation(Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, "pipeline/window_world_front"))
			.withShaderDefine("ALPHA_CUTOUT")
			.withDepthStencilState(WORLD_DEPTH)
			.build();

	private static final RenderPipeline WORLD_BACK = RenderPipeline.builder(WINDOW_SNIPPET)
			.withLocation(Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, "pipeline/window_world_back"))
			.withShaderDefine("ALPHA_CUTOUT")
			.withShaderDefine("NO_COLOR")
			.withDepthStencilState(WORLD_DEPTH)
			.build();

	private static final RenderPipeline OVERLAY_FRONT = RenderPipeline.builder(WINDOW_SNIPPET)
			.withLocation(Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, "pipeline/window_overlay_front"))
			.withShaderDefine("ALPHA_CUTOUT")
			.withDepthStencilState(OVERLAY_DEPTH)
			.build();

	private static final RenderPipeline OVERLAY_BACK = RenderPipeline.builder(WINDOW_SNIPPET)
			.withLocation(Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, "pipeline/window_overlay_back"))
			.withShaderDefine("ALPHA_CUTOUT")
			.withShaderDefine("NO_COLOR")
			.withDepthStencilState(OVERLAY_DEPTH)
			.build();

	private static DynamicUniformStorage<MvpUniform> uniformStorage = null;

	private static final List<PendingWindow> pending = new ArrayList<>();
	private static final List<DrawBatch> batches = new ArrayList<>();

	// Camera matrices for the frame, captured at COLLECT_SUBMITS - drawPending()
	// runs later (renderLevel return) with no LevelRenderContext in hand.
	private static final Matrix4f projection = new Matrix4f();
	private static final Matrix4f viewRotation = new Matrix4f();

	// Scene depth copied out of the main render target at END_MAIN, before Iris
	// composites and clears it. Used as the depth attachment in drawPending().
	private static GpuTexture capturedDepth = null;
	private static GpuTextureView capturedDepthView = null;

	private ShaderWindowPass() {
	}

	public static void register() {
		LevelRenderEvents.END_MAIN.register(ctx -> captureDepth());
	}

	public static void captureCamera(Matrix4fc proj, Matrix4fc viewRot) {
		projection.set(proj);
		viewRotation.set(viewRot);
	}

	public static void captureDepth() {
		// Nothing enqueued this frame - skip the full-screen depth copy.
		if(pending.isEmpty()) return;

		RenderTarget target = Minecraft.getInstance().getMainRenderTarget();
		GpuTexture sceneDepth = target.getDepthTexture();
		if(sceneDepth == null) {
			// Main depth gone at END_MAIN - capturedDepth keeps the previous
			// frame's content; drawPending guards against the size drift.
			return;
		}

		int width = sceneDepth.getWidth(0);
		int height = sceneDepth.getHeight(0);
		TextureFormat format = sceneDepth.getFormat();

		if(capturedDepth != null && (capturedDepth.getWidth(0) != width
				|| capturedDepth.getHeight(0) != height || capturedDepth.getFormat() != format)) {
			destroyCapturedDepth();
		}

		boolean created = capturedDepth == null;
		if(created) {
			// Match the render target depth texture's usage flags.
			int usage = GpuTexture.USAGE_COPY_DST | GpuTexture.USAGE_TEXTURE_BINDING | GpuTexture.USAGE_RENDER_ATTACHMENT;
			capturedDepth = RenderSystem.getDevice().createTexture(
					() -> "waylandcraft captured scene depth", usage, format, width, height, 1, 1);
			capturedDepthView = RenderSystem.getDevice().createTextureView(capturedDepth);
		}

		RenderSystem.getDevice().createCommandEncoder()
				.copyTextureToTexture(sceneDepth, capturedDepth, 0, 0, 0, 0, 0, width, height);
	}

	private static void destroyCapturedDepth() {
		if(capturedDepthView != null) capturedDepthView.close();
		if(capturedDepth != null) capturedDepth.close();
		capturedDepthView = null;
		capturedDepth = null;
	}

	/* pose is the model transform from the caller's PoseStack. World-space windows
	 * are in camera-relative world space; view-space (in-hand) windows are already
	 * in view space, so drawPending applies viewRotation only to world-space ones.
	 * Every submission is drawn: a window placed in the world and held in hand is
	 * submitted twice and must draw both the placed quad and the in-hand quad. */
	public static void enqueue(WindowFramebuffer framebuffer, Matrix4fc pose, boolean viewSpace, Vec3 tl, Vec3 bl, Vec3 br, Vec3 tr) {
		pending.add(new PendingWindow(framebuffer, new Matrix4f(pose), viewSpace, tl, bl, br, tr));
	}

	public static void drawPending() {
		if(pending.isEmpty()) return;

		try {
			RenderTarget target = Minecraft.getInstance().getMainRenderTarget();
			GpuTexture colorTex = target.getColorTexture();
			GpuTextureView color = target.getColorTextureView();
			if(colorTex == null || color == null) return;

			// Depth attachment for the post-composite pass is the scene depth
			// captured at END_MAIN, never target.getDepthTexture() - by here the
			// main depth has been cleared and overwritten by the hand pass.
			//
			// The pass FBO is built from the main color texture plus this depth
			// texture; OpenGL requires every attachment to share dimensions or
			// the FBO is incomplete and the GL backend drops the draw with no
			// exception. captureDepth() sizes capturedDepth to the main depth at
			// END_MAIN, but nothing guarantees the main target was not resized
			// between END_MAIN and here, so the sizes are checked. On a mismatch
			// (or a missing capture) the windows are drawn without a depth
			// attachment: they stay visible but lose scene occlusion for that
			// frame, which beats vanishing.
			GpuTextureView depth = capturedDepthView;
			if(depth != null && (capturedDepth.getWidth(0) != colorTex.getWidth(0)
					|| capturedDepth.getHeight(0) != colorTex.getHeight(0))) {
				depth = null;
			}

			ensureUniformStorage();

			for(PendingWindow window : pending) {
				GpuTextureView texture = window.framebuffer().getTextureView();
				if(texture == null) continue;

				Matrix4f mvp = new Matrix4f(projection);
				if(!window.viewSpace()) mvp.mul(viewRotation);
				mvp.mul(window.pose());
				GpuBufferSlice uniform = uniformStorage.writeUniform(new MvpUniform(mvp));

				RenderPipeline front = window.viewSpace() ? OVERLAY_FRONT : WORLD_FRONT;
				RenderPipeline back = window.viewSpace() ? OVERLAY_BACK : WORLD_BACK;
				batches.add(new DrawBatch(texture, uniform, front, buildQuad(window, false)));
				batches.add(new DrawBatch(texture, uniform, back, buildQuad(window, true)));
			}

			try(RenderPass pass = RenderSystem.getDevice().createCommandEncoder().createRenderPass(
					() -> "waylandcraft window post-composite", color, OptionalInt.empty(), depth, OptionalDouble.empty())) {
				for(DrawBatch batch : batches) {
					pass.setPipeline(batch.pipeline());
					pass.setUniform("window_mvp", batch.uniform());
					pass.bindTexture("Sampler0", batch.texture(), RenderUtils.WINDOW_SAMPLER.get());
					pass.setVertexBuffer(0, batch.geometry().vertexBuffer());
					pass.setIndexBuffer(batch.geometry().indexBuffer(), batch.geometry().indexType());
					pass.drawIndexed(0, 0, batch.geometry().indexCount(), 1);
				}
			}
		}
		finally {
			for(DrawBatch batch : batches) {
				batch.geometry().vertexBuffer().close();
			}
			batches.clear();
			pending.clear();
		}
	}

	/* Front quad winds CCW; the NO_COLOR back quad winds CW so culling shows
	 * exactly one face per viewing side, matching RenderUtils.FramebufferRenderInstance. */
	private static QuadGeometry buildQuad(PendingWindow w, boolean back) {
		Vec3[] corners;
		float[] uv;
		if(!back) {
			corners = new Vec3[] { w.tl(), w.bl(), w.br(), w.tr() };
			uv = new float[] { 0, 0, 0, 1, 1, 1, 1, 0 };
		}
		else {
			corners = new Vec3[] { w.tr(), w.br(), w.bl(), w.tl() };
			uv = new float[] { 1, 0, 1, 1, 0, 1, 0, 0 };
		}

		try(ByteBufferBuilder byteBuilder = new ByteBufferBuilder(DefaultVertexFormat.POSITION_TEX.getVertexSize() * 4)) {
			BufferBuilder builder = new BufferBuilder(byteBuilder, VertexFormat.Mode.QUADS, DefaultVertexFormat.POSITION_TEX);
			for(int i = 0; i < 4; i++) {
				Vec3 c = corners[i];
				builder.addVertex((float) c.x, (float) c.y, (float) c.z).setUv(uv[i * 2], uv[i * 2 + 1]);
			}

			try(MeshData mesh = builder.buildOrThrow()) {
				int indexCount = mesh.drawState().indexCount();
				RenderSystem.AutoStorageIndexBuffer indices = RenderSystem.getSequentialBuffer(VertexFormat.Mode.QUADS);
				GpuBuffer vertexBuffer = RenderSystem.getDevice().createBuffer(null, GpuBuffer.USAGE_VERTEX | GpuBuffer.USAGE_COPY_DST, mesh.vertexBuffer());
				GpuBuffer indexBuffer = indices.getBuffer(indexCount);
				return new QuadGeometry(vertexBuffer, indexBuffer, indexCount, indices.type());
			}
		}
	}

	private static void ensureUniformStorage() {
		if(uniformStorage == null) {
			uniformStorage = new DynamicUniformStorage<>("waylandcraft window mvp", MvpUniform.SIZE, 8);
		}
	}

	public static void endFrame() {
		if(uniformStorage != null) uniformStorage.endFrame();
	}

	private record PendingWindow(WindowFramebuffer framebuffer, Matrix4f pose, boolean viewSpace, Vec3 tl, Vec3 bl, Vec3 br, Vec3 tr) {
	}

	private record QuadGeometry(GpuBuffer vertexBuffer, GpuBuffer indexBuffer, int indexCount, VertexFormat.IndexType indexType) {
	}

	private record DrawBatch(GpuTextureView texture, GpuBufferSlice uniform, RenderPipeline pipeline, QuadGeometry geometry) {
	}

	private record MvpUniform(Matrix4fc mvp) implements DynamicUniform {

		static final int SIZE = new Std140SizeCalculator().putMat4f().get();

		@Override
		public void write(ByteBuffer byteBuffer) {
			Std140Builder.intoBuffer(byteBuffer).putMat4f(mvp);
		}

	}

}
