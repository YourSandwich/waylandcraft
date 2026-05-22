package dev.evvie.waylandcraft.mixin;

import org.jetbrains.annotations.Nullable;
import org.joml.Matrix3x2fStack;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

import com.mojang.blaze3d.pipeline.RenderPipeline;

import dev.evvie.waylandcraft.CursorShape;
import dev.evvie.waylandcraft.WaylandCraft;
import dev.evvie.waylandcraft.WaylandCraft.KeyboardCaptureMode;
import dev.evvie.waylandcraft.bridge.IconSurface;
import dev.evvie.waylandcraft.render.RenderUtils;
import dev.evvie.waylandcraft.render.WindowFramebuffer;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Gui;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.resources.Identifier;
import net.minecraft.world.phys.Vec3;

@Mixin(Gui.class)
public class GuiMixin {
	
	private static final Identifier TLBR_DIAGONAL_CROSSHAIR = Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, "crosshair/tlbr_diagonal");
	private static final Identifier TRBL_DIAGONAL_CROSSHAIR = Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, "crosshair/trbl_diagonal");
	private static final Identifier LEFT_RIGHT_CROSSHAIR = Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, "crosshair/left_right");
	private static final Identifier TOP_BOTTOM_CROSSHAIR = Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, "crosshair/top_bottom");
	
	private static final Identifier HELP_CROSSHAIR = Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, "crosshair/help");
	private static final Identifier MOVE_CROSSHAIR = Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, "crosshair/move");
	private static final Identifier POINTER_CROSSHAIR = Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, "crosshair/pointer");
	private static final Identifier TEXT_CROSSHAIR = Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, "crosshair/text");
	private static final Identifier VTEXT_CROSSHAIR = Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, "crosshair/vtext");
	private static final Identifier WAIT_CROSSHAIR = Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, "crosshair/wait");
	private static final Identifier ZOOM_IN_CROSSHAIR = Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, "crosshair/zoom_in");
	private static final Identifier ZOOM_OUT_CROSSHAIR = Identifier.fromNamespaceAndPath(WaylandCraft.MOD_ID, "crosshair/zoom_out");
	
	@Redirect(method = "extractCrosshair", at = @At(value = "INVOKE", target = "Lnet/minecraft/client/gui/GuiGraphicsExtractor;blitSprite(Lcom/mojang/blaze3d/pipeline/RenderPipeline;Lnet/minecraft/resources/Identifier;IIII)V", ordinal = 0))
	public void crosshairBlitSprite(GuiGraphicsExtractor context, RenderPipeline pipeline, Identifier original, int x, int y, int width, int height) {
		// The crosshair only turns into a client cursor in Alt+G desktop
		// capture. Every other mode keeps the normal Minecraft crosshair.
		if(WaylandCraft.instance.keyboardCaptureMode != KeyboardCaptureMode.DESKTOP) {
			context.blitSprite(pipeline, original, x, y, width, height);
			return;
		}

		// The cursor centre. Normally the crosshair position - the camera-look
		// ray-cast against the focused window - but in Alt+G desktop capture the
		// camera is locked and the mouse drives the cursor, so it sits wherever
		// that on-window position projects onto the screen.
		Vec3 desktop = WaylandCraft.instance.desktopCursorScreenPos();
		float centerX = desktop != null ? (float) desktop.x : x + width / 2.0f;
		float centerY = desktop != null ? (float) desktop.y : y + height / 2.0f;

		// A client cursor surface, a named cursor-shape, and the default
		// crosshair all draw at the cursor centre.
		if(renderCursorSurface(context, centerX, centerY)) return;

		CursorShape cursor = WaylandCraft.instance.cursorShape;

		// A client hiding the cursor (null set_cursor surface) reports HIDE -
		// draw nothing, not the default crosshair.
		if(cursor == CursorShape.HIDE) return;

		Identifier crosshair = crosshairForCursor(cursor);

		// No client cursor surface and no dedicated shape sprite - the default
		// arrow, or an unknown shape. Fall back to the Minecraft crosshair so
		// the pointer stays visible in desktop capture.
		if(crosshair == null) crosshair = original;

		context.blitSprite(pipeline, crosshair, Math.round(centerX - width / 2.0f), Math.round(centerY - height / 2.0f), width, height);
	}

	/* Draw the client-provided cursor surface at the pointer position, with the
	 * cursor hotspot pinned to that point. The framebuffer holds real Wayland
	 * pixels, so it is scaled down by the GUI scale to keep its native size,
	 * matching how the dnd icon is drawn. Returns true when the surface cursor
	 * was drawn, so the caller skips the cursor-shape fallback. A hidden cursor
	 * (null set_cursor surface) clears cursorIcon, so it returns false and the
	 * caller handles the HIDE shape instead.
	 */
	private boolean renderCursorSurface(GuiGraphicsExtractor context, float pointerX, float pointerY) {
		// Only while the pointer is actually over a Wayland surface: a cursor
		// surface set earlier must not keep drawing once the pointer leaves.
		if(!WaylandCraft.instance.pointerOnSurface) return false;

		IconSurface cursor = WaylandCraft.instance.bridge.cursorIcon;
		if(cursor == null || cursor.framebuffer == null) return false;

		WindowFramebuffer buf = cursor.framebuffer;
		int guiScale = (int) Minecraft.getInstance().getWindow().getGuiScale();

		// Buffer-local offset of the hotspot: the surface tree's own xoff/yoff
		// plus the client-given hotspot, all in Wayland pixels.
		int hotspotX = WaylandCraft.instance.bridge.getCursorHotspotX() + buf.getXOff();
		int hotspotY = WaylandCraft.instance.bridge.getCursorHotspotY() + buf.getYOff();

		Matrix3x2fStack stack = context.pose();
		stack.pushMatrix();
		stack.translate(pointerX, pointerY);
		stack.scale(1.0f / guiScale, 1.0f / guiScale);
		RenderUtils.renderFramebuffer2D(context, buf, -hotspotX, -hotspotY, buf.getWidth(), buf.getHeight());
		stack.popMatrix();
		return true;
	}
	
	private @Nullable Identifier crosshairForCursor(@Nullable CursorShape cursor) {
		if(cursor == null) return null;
		
		switch(cursor) {
		case HIDE: return null; // Handled by the caller; never draws a crosshair
		case DEFAULT: return null;
		case HELP: return HELP_CROSSHAIR;
		case POINTER: return POINTER_CROSSHAIR;
		case WAIT: return WAIT_CROSSHAIR;
		case TEXT: return TEXT_CROSSHAIR;
		case VERTICAL_TEXT: return VTEXT_CROSSHAIR;
		case E_RESIZE: return LEFT_RIGHT_CROSSHAIR;
		case N_RESIZE: return TOP_BOTTOM_CROSSHAIR;
		case NE_RESIZE: return TRBL_DIAGONAL_CROSSHAIR;
		case NW_RESIZE: return TLBR_DIAGONAL_CROSSHAIR;
		case S_RESIZE: return TOP_BOTTOM_CROSSHAIR;
		case SE_RESIZE: return TLBR_DIAGONAL_CROSSHAIR;
		case SW_RESIZE: return TRBL_DIAGONAL_CROSSHAIR;
		case W_RESIZE: return LEFT_RIGHT_CROSSHAIR;
		case EW_RESIZE: return LEFT_RIGHT_CROSSHAIR;
		case NS_RESIZE: return TOP_BOTTOM_CROSSHAIR;
		case NESW_RESIZE: return TRBL_DIAGONAL_CROSSHAIR;
		case NWSE_RESIZE: return TLBR_DIAGONAL_CROSSHAIR;
		case COL_RESIZE: return LEFT_RIGHT_CROSSHAIR;
		case ROW_RESIZE: return TOP_BOTTOM_CROSSHAIR;
		case ZOOM_IN: return ZOOM_IN_CROSSHAIR;
		case ZOOM_OUT: return ZOOM_OUT_CROSSHAIR;
		case ALL_RESIZE: return MOVE_CROSSHAIR;
		default: return null;
		}
	}
	
}
