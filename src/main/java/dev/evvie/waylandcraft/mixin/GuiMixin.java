package dev.evvie.waylandcraft.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

import dev.evvie.waylandcraft.WaylandCraft;
import net.minecraft.client.gui.Gui;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.resources.ResourceLocation;

@Mixin(Gui.class)
public class GuiMixin {
	
	private static final ResourceLocation TLBR_DIAGONAL_CROSSHAIR = new ResourceLocation(WaylandCraft.MOD_ID, "crosshair/tlbr_diagonal");
	private static final ResourceLocation TRBL_DIAGONAL_CROSSHAIR = new ResourceLocation(WaylandCraft.MOD_ID, "crosshair/trbl_diagonal");
	private static final ResourceLocation LEFT_RIGHT_CROSSHAIR = new ResourceLocation(WaylandCraft.MOD_ID, "crosshair/left_right");
	private static final ResourceLocation TOP_BOTTOM_CROSSHAIR = new ResourceLocation(WaylandCraft.MOD_ID, "crosshair/top_bottom");
	
	@Redirect(method = "renderCrosshair", at = @At(value = "INVOKE", target = "Lnet/minecraft/client/gui/GuiGraphics;blitSprite(Lnet/minecraft/resources/ResourceLocation;IIII)V", ordinal = 0))
	public void crosshairBlitSprite(GuiGraphics context, ResourceLocation original, int x, int y, int width, int height) {
		ResourceLocation crosshair;
		
		int select = (int) ((System.currentTimeMillis() / 500) % 5);
		switch(select) {
		case 0: crosshair = original; break;
		case 1: crosshair = TLBR_DIAGONAL_CROSSHAIR; break;
		case 2: crosshair = TRBL_DIAGONAL_CROSSHAIR; break;
		case 3: crosshair = LEFT_RIGHT_CROSSHAIR; break;
		case 4: crosshair = TOP_BOTTOM_CROSSHAIR; break;
		default: crosshair = original; break;
		}
		
		context.blitSprite(crosshair, x, y, width, height);
	}
	
}
