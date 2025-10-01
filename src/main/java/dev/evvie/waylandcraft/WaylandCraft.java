package dev.evvie.waylandcraft;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import com.mojang.blaze3d.systems.RenderSystem;

import dev.evvie.waylandcraft.bridge.WLCToplevel;
import dev.evvie.waylandcraft.bridge.WaylandCraftBridge;
import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.api.ModInitializer;
import net.fabricmc.fabric.api.client.rendering.v1.WorldRenderContext;
import net.fabricmc.fabric.api.client.rendering.v1.WorldRenderEvents;
import net.minecraft.client.Minecraft;
import net.minecraft.network.chat.Component;
import net.minecraft.world.phys.Vec2;
import net.minecraft.world.phys.Vec3;

public class WaylandCraft implements ModInitializer, ClientModInitializer {
	public static final String MOD_ID = "waylandcraft";
	public static final Logger LOGGER = LoggerFactory.getLogger(MOD_ID);
	
	private WaylandCraftBridge bridge = null;
	
	@Override
	public void onInitialize() {
	}
	
	private Vec2 getToplevelDimensions(WLCToplevel toplevel) {
		BufferTexture buf = toplevel.getSurfaceTree().getBuffer();
		if(buf == null) return new Vec2(0, 0);
		float width = buf.width;
		float height = buf.height;
		return new Vec2(width / 500, height / 500);
	}

	private void renderToplevelAt(WorldRenderContext ctx, WLCToplevel toplevel, Vec3 pos) {
		BufferTexture buf = toplevel.getSurfaceTree().getBuffer();
		if(buf == null) return;
		
		Vec2 size = getToplevelDimensions(toplevel);
		RenderUtils.drawTexturedQuad(ctx.camera(), buf.getId(),
				pos, pos.add(size.x, 0, 0), pos.add(size.x, size.y, 0), pos.add(0, size.y, 0),
				new Vec2(0, 1), new Vec2(1, 1), new Vec2(1, 0), new Vec2(0, 0));
	}
	
	@Override
	public void onInitializeClient() {
		LOGGER.info("Initializing WaylandCraft");
		
		WorldRenderEvents.END.register(context -> {
			if(bridge == null) {
				bridge = WaylandCraftBridge.start();
				String socket = bridge.getSocket();
				Minecraft.getInstance().gui.getChat().addMessage(Component.literal("Server started on " + socket));
			}
			bridge.update();
			
			RenderSystem.enableDepthTest();
			Vec3 pos = new Vec3(-250, 65, -500);
			WLCToplevel[] toplevels = bridge.getToplevels();
			for(WLCToplevel toplevel : toplevels) {
				renderToplevelAt(context, toplevel, pos);
				Vec2 size = getToplevelDimensions(toplevel);
				pos = pos.add(size.x, 0, 0);
			}
		});
	}
}