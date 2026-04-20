package dev.evvie.waylandcraft.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.Redirect;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import com.llamalad7.mixinextras.sugar.Local;
import com.mojang.blaze3d.vertex.PoseStack;

import dev.evvie.waylandcraft.WaylandCraft;
import dev.evvie.waylandcraft.bridge.WLCToplevel;
import dev.evvie.waylandcraft.item.WindowItem;
import dev.evvie.waylandcraft.render.IMyItemFrameRenderState;
import net.minecraft.client.renderer.MultiBufferSource;
import net.minecraft.client.renderer.entity.ItemFrameRenderer;
import net.minecraft.client.renderer.entity.state.ItemFrameRenderState;
import net.minecraft.client.renderer.item.ItemStackRenderState;
import net.minecraft.client.resources.model.BlockStateModelLoader;
import net.minecraft.client.resources.model.ModelResourceLocation;
import net.minecraft.world.entity.decoration.ItemFrame;

@Mixin(ItemFrameRenderer.class)
public class ItemFrameRendererMixin {
	
	@Redirect(method = "render", at = @At(value = "INVOKE", target = "Lnet/minecraft/client/renderer/item/ItemStackRenderState;render(Lcom/mojang/blaze3d/vertex/PoseStack;Lnet/minecraft/client/renderer/MultiBufferSource;II)V"))
	public void renderItem(ItemStackRenderState itemStackRenderState, PoseStack poseStack, MultiBufferSource multiBufferSource, int light, int overlay, @Local ItemFrameRenderState itemFrameRenderState) {
		WLCToplevel toplevel = ((IMyItemFrameRenderState) itemFrameRenderState).getToplevel();
		
		if(toplevel == null) {
			itemStackRenderState.render(poseStack, multiBufferSource, light, overlay);
			return;
		}
		
		WaylandCraft.instance.windowInItemFrameRenderer.render(toplevel, poseStack, multiBufferSource);
	}
	
	@Inject(method = "extractRenderState", at = @At("TAIL"))
	public void extractRenderState(ItemFrame itemFrame, ItemFrameRenderState itemFrameRenderState, float f, CallbackInfo info) {
		WLCToplevel toplevel = WindowItem.getToplevel(itemFrame.getItem());
		((IMyItemFrameRenderState) itemFrameRenderState).setToplevel(toplevel);
	}
	
	@Inject(method = "getFrameModelResourceLocation", at = @At("HEAD"), cancellable = true)
	private static void redirectGetFrameModelResourceLocation(ItemFrameRenderState itemFrameRenderState, CallbackInfoReturnable<ModelResourceLocation> info) {
		WLCToplevel toplevel = ((IMyItemFrameRenderState) itemFrameRenderState).getToplevel();
		if(toplevel == null) return;
		
		info.setReturnValue(itemFrameRenderState.isGlowFrame ? BlockStateModelLoader.GLOW_MAP_FRAME_LOCATION : BlockStateModelLoader.MAP_FRAME_LOCATION);
		info.cancel();
	}
	
}
